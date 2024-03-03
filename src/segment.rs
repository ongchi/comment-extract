// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Write};
use std::iter::zip;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Error;
use rustdoc_types::{Crate, Id, Item, ItemEnum, ItemKind, ItemSummary};

use crate::repr::Repr;
use crate::utils::hide_code_block_lines;
use crate::{Config, Package};

#[derive(Debug)]
pub struct ExportOption {
    package: Package,
    module_path: Option<PathBuf>,
    kind: ItemKind,
}

#[derive(Debug)]
pub struct SegmentCollections {
    output_root: PathBuf,
    items: Vec<Rc<CachedItem>>,
}

impl SegmentCollections {
    pub fn extract(&self) -> Result<(), Error> {
        for item in &self.items {
            let (root, filename) = item
                .path()
                .as_slice()
                .split_last()
                .map(|(name, path)| {
                    let root = self.output_root.join(PathBuf::from_iter(path));
                    let file = root.join(format!("{}.md", name));
                    (root, file)
                })
                .unwrap();

            create_dir_all(&root)?;

            let mut file = File::create(filename)?;
            file.write_all(item.repr(item).as_bytes())?;
        }

        Ok(())
    }
}

impl TryFrom<Config> for SegmentCollections {
    type Error = Error;

    fn try_from(value: Config) -> Result<Self, Self::Error> {
        let manifest_path = value.manifest_path.as_deref().unwrap_or("Cargo.toml");
        let output_root = PathBuf::from(value.output_path);
        let mut packages = HashMap::new();
        let mut extract_options = vec![];

        for package in value.packages {
            if packages.get(&package.name).is_none() {
                let builder = rustdoc_json::Builder::default()
                    .manifest_path(manifest_path)
                    .package(&package.name)
                    .toolchain("nightly")
                    .all_features(true)
                    .clear_target_dir();

                let json_path = builder.build()?;
                let file = File::open(json_path)?;
                let reader = BufReader::new(file);
                let crate_: Crate = serde_json::from_reader(reader)?;

                packages.insert(package.name.clone(), crate_);
            }

            let kind = serde_plain::from_str(&package.kind)?;
            let module_path = package
                .module_path
                .as_deref()
                .map(|s| s.split("::").collect());

            extract_options.push(ExportOption {
                package,
                module_path,
                kind,
            });
        }

        let pool = Rc::new(ItemPool {
            crates: packages,
            cached_items: RefCell::new(HashMap::new()),
            extract_items: RefCell::new(vec![]),
        });

        // Collect items to be extract
        let mut items = vec![];
        for option in extract_options {
            let crate_ = pool.crates.get(&option.package.name).unwrap();
            items.extend(
                crate_
                    .index
                    .keys()
                    .filter_map(|id| crate_.paths.get(id).map(|summ| (id, summ)))
                    .filter(|(_, summ)| summ.kind == option.kind)
                    .filter(|(_, summ)| {
                        option
                            .module_path
                            .as_ref()
                            .map(|p| summ.path.iter().collect::<PathBuf>().starts_with(p))
                            .unwrap_or(true)
                    })
                    .flat_map(|(id, _)| {
                        let id = ItemId::new(&option.package.name, id);
                        let item = pool.clone().get(&id);
                        let methods = item.associated_methods();
                        methods.into_iter().chain([item])
                    }),
            )
        }

        pool.extract_items.borrow_mut().extend(items.clone());

        Ok(Self { output_root, items })
    }
}

#[derive(Debug)]
pub struct ItemPool {
    crates: HashMap<String, Crate>,
    cached_items: RefCell<HashMap<ItemId, Rc<CachedItem>>>,
    extract_items: RefCell<Vec<Rc<CachedItem>>>,
}

impl ItemPool {
    pub fn get(self: Rc<Self>, id: &ItemId) -> Rc<CachedItem> {
        self.insert_with_path(id, None)
    }

    fn insert_with_path(self: Rc<Self>, id: &ItemId, path: Option<Vec<String>>) -> Rc<CachedItem> {
        let cached_item = self.cached_items.borrow().get(id).cloned();

        if let Some(cached_item) = cached_item {
            cached_item
        } else {
            let item = CachedItem::new(self.clone(), id.clone(), path);
            self.cached_items
                .borrow_mut()
                .insert(id.clone(), item.clone());
            item
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ItemId {
    pub pkg: String,
    pub id: Id,
}

impl ItemId {
    pub fn new(pkg: &str, id: &Id) -> Self {
        Self {
            pkg: pkg.to_string(),
            id: id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachedItem {
    pub pool: Rc<ItemPool>,
    pub id: ItemId,
    path: Option<Vec<String>>,
    external_link: OnceCell<String>,
}

impl CachedItem {
    fn new(pool: Rc<ItemPool>, id: ItemId, path: Option<Vec<String>>) -> Rc<Self> {
        Rc::new(Self {
            pool,
            id: id.clone(),
            path,
            external_link: OnceCell::new(),
        })
    }

    // Associated methods does not have `ItemSummary`, which means we needs to grab path infomation
    // from parent.
    pub fn associated_methods(&self) -> Vec<Rc<CachedItem>> {
        if let Some(item) = self.item() {
            let crate_ = self.pool.crates.get(&self.id.pkg).unwrap();
            match &item.inner {
                ItemEnum::Struct(ref struct_) => struct_
                    .impls
                    .iter()
                    .filter_map(|id| crate_.index.get(id))
                    .filter_map(|item| match item.inner {
                        ItemEnum::Impl(ref impl_) => match impl_.trait_ {
                            Some(_) => None,
                            None => Some(&impl_.items),
                        },
                        _ => None,
                    })
                    .flatten()
                    .map(|id| {
                        let item_id = ItemId::new(&self.id.pkg, id);
                        let method_item = crate_.index.get(id).unwrap();
                        (item_id, method_item)
                    })
                    .map(|(item_id, method_item)| {
                        let name = method_item.name.as_deref();
                        let path = (self.path().into_iter().chain(name))
                            .map(|p| p.to_string())
                            .collect();
                        (item_id, path)
                    })
                    .map(|(item_id, path)| self.pool.clone().insert_with_path(&item_id, Some(path)))
                    .collect(),
                _ => vec![],
            }
        } else {
            vec![]
        }
    }

    pub fn item(&self) -> Option<&Item> {
        self.pool
            .crates
            .get(&self.id.pkg)
            .unwrap()
            .index
            .get(&self.id.id)
    }

    pub fn item_summary(&self) -> Option<&ItemSummary> {
        self.pool
            .crates
            .get(&self.id.pkg)
            .unwrap()
            .paths
            .get(&self.id.id)
    }

    pub fn kind(&self) -> &ItemKind {
        match self.item_summary() {
            Some(summ) => &summ.kind,
            // Associated method of a struct does not have an `ItemSummary`. Since a method will be
            // no different to a function during document generation.
            // Simply returning an `ItemKind::Function` in this case will be sufficient.
            None => &ItemKind::Function,
        }
    }

    pub fn name(&self) -> &str {
        self.item()
            .and_then(|item| item.name.as_deref())
            .or(self
                .item_summary()
                .and_then(|summ| summ.path.last())
                .map(|name| name.as_str()))
            .unwrap()
    }

    fn path(&self) -> Vec<&str> {
        self.item_summary()
            .map(|summ| summ.path.as_ref())
            .or(self.path.as_ref())
            .map(|path| path.iter().map(|p| p.as_str()))
            .unwrap()
            .collect()
    }
}

impl CachedItem {
    fn html_root_url(&self) -> String {
        let root_url = (self.item().map(|item| item.crate_id))
            .or(self.item_summary().map(|summ| summ.crate_id))
            .and_then(|ext_crate_id| {
                (self.pool.crates)
                    .get(&self.id.pkg)
                    .map(|crate_| (crate_, ext_crate_id))
            })
            .and_then(|(crate_, ext_crate_id)| crate_.external_crates.get(&ext_crate_id))
            .and_then(|ext_crate| ext_crate.html_root_url.as_deref());

        match root_url {
            Some(url) => url.to_string(),
            None => {
                let pkg = self.path().first().cloned().unwrap();
                if self.pool.crates.keys().any(|k| k == pkg) {
                    let crate_version = (self.pool.crates.get(&self.id.pkg))
                        .map(|crate_| crate_.crate_version.as_deref().unwrap())
                        .unwrap();
                    format!("https://docs.rs/{}/{}/", pkg, crate_version)
                } else {
                    // For external crates
                    format!("https://docs.rs/{}/latest/", pkg)
                }
            }
        }
    }

    pub fn external_link(&self) -> &str {
        self.external_link.get_or_init(|| {
            format!(
                "{}{}/{}.{}.html",
                self.html_root_url(),
                self.path()
                    .split_last()
                    .map(|(_, path)| path.join("/"))
                    .unwrap(),
                serde_plain::to_string(self.kind()).unwrap(),
                self.name()
            )
        })
    }

    fn relative_to(&self, other: &Self) -> Vec<String> {
        let left = self.path();
        let left = (left.split_last().map(|(_, path)| path)).unwrap();
        let right = other.path();
        let right = (right.split_last().map(|(_, path)| path)).unwrap();
        let d = zip(left, right).map(|(l, r)| (l == r) as usize).sum();

        (0..(left.len() - d))
            .map(|_| "..")
            .chain(right.iter().cloned().skip(d))
            .map(|p| p.to_string())
            .collect()
    }

    pub fn cross_ref(&self, to: &Self) -> String {
        self.relative_to(to)
            .into_iter()
            .map(|p| p.to_string())
            .chain([format!("{}.md", to.name())])
            .collect::<Vec<String>>()
            .join("/")
    }

    pub fn docs(&self) -> String {
        hide_code_block_lines(
            self.item()
                .and_then(|item| item.docs.as_deref())
                .unwrap_or(""),
        )
    }
}
