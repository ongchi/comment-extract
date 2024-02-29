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

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Write};
use std::iter::zip;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{anyhow, Error};
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, Id, Item, ItemEnum, ItemKind, ItemSummary, Term,
    TraitBoundModifier, Type, TypeBinding, TypeBindingKind,
};

use crate::doc_traits::{CrossRef, ModulePath, Name, RelativeTo, Repr};
use crate::utils::{caption, hide_code_block_lines};
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
                let file = File::open(json_path).map_err(|e| anyhow!(e))?;
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
    fn get(self: Rc<Self>, id: &ItemId) -> Rc<CachedItem> {
        let cached_item = self.cached_items.borrow().get(id).cloned();

        if let Some(cached_item) = cached_item {
            cached_item
        } else {
            let item = CachedItem::new(self.clone(), id.clone());
            self.cached_items
                .borrow_mut()
                .insert(id.clone(), item.clone());
            item
        }
    }

    fn insert_with_path(self: Rc<Self>, id: &ItemId, path: Vec<String>) -> Rc<CachedItem> {
        let cached_item = self.cached_items.borrow().get(id).cloned();

        if let Some(cached_item) = cached_item {
            cached_item
        } else {
            let item = CachedItem::new_with_path(self.clone(), id.clone(), Some(path));
            self.cached_items
                .borrow_mut()
                .insert(id.clone(), item.clone());
            item
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct ItemId {
    pkg: String,
    id: Id,
}

impl ItemId {
    fn new(pkg: &str, id: &Id) -> Self {
        Self {
            pkg: pkg.to_string(),
            id: id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachedItem {
    pool: Rc<ItemPool>,
    id: ItemId,
    path: Option<Vec<String>>,
}

impl CachedItem {
    fn new(pool: Rc<ItemPool>, id: ItemId) -> Rc<Self> {
        Self::new_with_path(pool, id, None)
    }

    fn new_with_path(pool: Rc<ItemPool>, id: ItemId, path: Option<Vec<String>>) -> Rc<Self> {
        Rc::new(Self {
            pool: pool.clone(),
            id: id.clone(),
            path,
        })
    }

    fn associated_methods(&self) -> Vec<Rc<CachedItem>> {
        if let Some(item) = self.item() {
            let crate_ = self.pool.crates.get(&self.id.pkg).unwrap();
            match &item.inner {
                ItemEnum::Struct(ref s) => s
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
                        let id = ItemId::new(&self.id.pkg, id);
                        let method_item = crate_.index.get(&id.id).unwrap();
                        self.pool.clone().insert_with_path(
                            &id,
                            self.path()
                                .iter()
                                .chain([&method_item.name.clone().unwrap()])
                                .cloned()
                                .collect(),
                        )
                    })
                    .collect(),
                _ => vec![],
            }
        } else {
            vec![]
        }
    }

    fn item(&self) -> Option<&Item> {
        self.pool
            .crates
            .get(&self.id.pkg)
            .unwrap()
            .index
            .get(&self.id.id)
    }

    fn item_summary(&self) -> Option<&ItemSummary> {
        self.pool
            .crates
            .get(&self.id.pkg)
            .unwrap()
            .paths
            .get(&self.id.id)
    }

    fn kind(&self) -> &ItemKind {
        match self.item_summary() {
            Some(summ) => &summ.kind,
            None => &ItemKind::Function,
        }
    }

    fn crate_version(&self) -> &str {
        self.pool
            .crates
            .get(&self.id.pkg)
            .map(|c| c.crate_version.as_deref().unwrap())
            .unwrap()
    }

    fn url_path(&self) -> String {
        self.path()
            .as_slice()
            .split_last()
            .map(|(_, path)| path.join("/"))
            .unwrap()
    }

    fn html_root_url(&self) -> String {
        let root_url = self.pool.crates.get(&self.id.pkg).and_then(|c| {
            c.external_crates
                .get(
                    &(self.item())
                        .map(|item| item.crate_id)
                        .or(self.item_summary().map(|summ| summ.crate_id))
                        .unwrap(),
                )
                .and_then(|c| c.html_root_url.as_deref())
        });

        match root_url {
            Some(url) => url.to_string(),
            None => {
                let pkg = (self.path().iter()).next().cloned().unwrap();
                if self.pool.crates.keys().any(|k| k == &pkg) {
                    format!("https://docs.rs/{}/{}/", pkg, self.crate_version())
                } else {
                    // For external crates
                    format!("https://docs.rs/{}/latest/", pkg)
                }
            }
        }
    }

    fn external_link(&self) -> String {
        format!(
            "{}{}/{}.{}.html",
            self.html_root_url(),
            self.url_path(),
            serde_plain::to_string(self.kind()).unwrap(),
            self.name()
        )
    }

    fn docs(&self) -> String {
        hide_code_block_lines(
            self.item()
                .and_then(|item| item.docs.as_deref())
                .unwrap_or(""),
        )
    }
}

impl Name for CachedItem {
    fn name(&self) -> &str {
        self.item()
            .and_then(|item| item.name.as_deref())
            .or(self
                .item_summary()
                .and_then(|summ| summ.path.last())
                .map(|s| s.as_str()))
            .unwrap()
    }
}

impl ModulePath for CachedItem {
    fn path(&self) -> &Vec<String> {
        self.item_summary()
            .map(|summ| &summ.path)
            .or(self.path.as_ref())
            .unwrap()
    }
}

impl<T> RelativeTo<T> for CachedItem
where
    T: ModulePath,
{
    fn relative_to(&self, other: &T) -> Vec<String> {
        let left = (self.path().as_slice().split_last().map(|(_, path)| path)).unwrap();
        let right = (other.path().as_slice().split_last().map(|(_, path)| path)).unwrap();

        let mut d = 0;
        for (l, r) in zip(left, right) {
            if l == r {
                d += 1
            }
        }

        (0..(left.len() - d))
            .map(|_| "..".to_string())
            .chain(right.iter().skip(d).cloned())
            .collect()
    }
}

impl Repr for CachedItem {
    fn repr(&self, _root: &CachedItem) -> String {
        match self.kind() {
            ItemKind::Function => {
                let name = self.name();
                format!(
                    r#"# {}

<dl>
    <dt class="sig">
    <span class="sig-name">
        <span class="pre">{}</span>
    </span>
    {}
    </dt>
</dl>

{}
"#,
                    name,
                    name,
                    self.item().unwrap().inner.repr(self),
                    self.docs()
                )
            }

            ItemKind::Struct => {
                let methods = self
                    .associated_methods()
                    .into_iter()
                    .map(|method| {
                        format!(
                            "| {} | {} |",
                            self.cross_ref_md(&method),
                            caption(method.item().unwrap())
                        )
                    })
                    .collect::<Vec<String>>()
                    .join("\n");

                if !methods.is_empty() {
                    format!(
                        "# {}\n\n{}\n\n# Methods\n| Method | Description |\n| --- | --- |\n{}",
                        self.name(),
                        self.docs(),
                        methods
                    )
                } else {
                    format!("# {}\n\n{}", self.name(), self.docs())
                }
            }

            _ => unimplemented!("Unimplemented ItemKind: {:?}", self),
        }
    }
}

impl Repr for ItemEnum {
    fn repr(&self, root: &CachedItem) -> String {
        match self {
            ItemEnum::Function(func) => {
                format!(
                    r#"<span class="sig-paren">(</span>
{}
<span class="sig-paren">)</span>
{}"#,
                    func.decl
                        .inputs
                        .iter()
                        .map(|(name, type_)| format!(
                            r#"<em class="sig-param n">
    <span class="pre">{}</span>: <span class="pre">{}</span>
</em>"#,
                            name,
                            type_.repr(root)
                        ))
                        .collect::<Vec<String>>()
                        .join(", "),
                    func.decl
                        .output
                        .as_ref()
                        .map(|type_| format!(" → {}", type_.repr(root)))
                        .unwrap_or("".to_string())
                )
            }
            _ => unimplemented!("Unimplemented ItemEnum: {:?}", self),
        }
    }
}

impl Repr for Type {
    fn repr(&self, root: &CachedItem) -> String {
        match self {
            Type::Primitive(p) => format!(
                "<a href=\"https://doc.rust-lang.org/std/primitive.{}.html\">{}</a>",
                p, p
            ),

            Type::ResolvedPath(path) => path.repr(root),

            Type::DynTrait(dyn_trait) => format!(
                "dyn {}",
                dyn_trait
                    .traits
                    .iter()
                    .map(|poly_trait| {
                        format!(
                            "{}{}",
                            if !poly_trait.generic_params.is_empty() {
                                unimplemented!("Unimplemented: Higher-Rank Trait Bounds")
                            } else {
                                ""
                            },
                            &poly_trait.trait_.repr(root)
                        )
                    })
                    .chain(dyn_trait.lifetime.iter().map(|t| t.to_string()))
                    .collect::<Vec<String>>()
                    .join(" + ")
            ),

            Type::Generic(t) => t.clone(),

            Type::BorrowedRef {
                lifetime,
                mutable,
                type_,
            } => {
                format!(
                    "&{}{}{}",
                    lifetime
                        .as_ref()
                        .map(|a| format!("{} ", a))
                        .unwrap_or("".to_string()),
                    if *mutable { "mut " } else { "" },
                    type_.repr(root)
                )
            }

            Type::Tuple(types) => format!(
                "({})",
                types
                    .iter()
                    .map(|type_| type_.repr(root))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),

            Type::Slice(slice) => format!("[{}]", slice.repr(root)),

            Type::Array { type_, len } => {
                format!("[{}: {}]", type_.repr(root), len)
            }

            Type::ImplTrait(bounds) => {
                format!(
                    "impl {}",
                    bounds
                        .iter()
                        .map(|bound| bound.repr(root))
                        .collect::<Vec<String>>()
                        .join(" + ")
                )
            }

            unknown => unimplemented!("Unimplemented Type: {:?}", unknown),
        }
    }
}

impl Repr for TypeBinding {
    fn repr(&self, root: &CachedItem) -> String {
        format!(
            "{}{}{}",
            self.name,
            &self.args.repr(root),
            match &self.binding {
                TypeBindingKind::Equality(term) => {
                    match term {
                        Term::Type(type_) => type_.repr(root),
                        Term::Constant(c) => {
                            unimplemented!("Unimplemented TypeBindingKind: {:?}", c)
                        }
                    }
                }
                TypeBindingKind::Constraint(c) =>
                    unimplemented!("Unimplemented TypeBindingKing: {:?}", c),
            }
        )
    }
}

impl Repr for GenericArgs {
    fn repr(&self, root: &CachedItem) -> String {
        match self {
            GenericArgs::AngleBracketed { args, bindings } => {
                if !args.is_empty() || !bindings.is_empty() {
                    format!(
                        "&lt;{}&gt;",
                        args.iter()
                            .map(|arg| match arg {
                                GenericArg::Lifetime(a) => a.clone(),
                                GenericArg::Type(type_) => type_.repr(root),
                                unknown =>
                                    unimplemented!("Unimplemented GenericArg: {:?}", unknown),
                            })
                            .chain(bindings.iter().map(|bind| bind.repr(root)))
                            .collect::<Vec<String>>()
                            .join(", ")
                    )
                } else {
                    "".to_string()
                }
            }
            _ => unimplemented!("Unimplemented GenericArgs: {:?}", self),
        }
    }
}

impl Repr for rustdoc_types::Path {
    fn repr(&self, root: &CachedItem) -> String {
        let id = ItemId::new(&root.id.pkg, &self.id);
        let item = root.pool.clone().get(&id);

        format!(
            "<a href=\"{}\">{}</a>{}",
            item.external_link(),
            item.name(),
            self.args
                .as_deref()
                .map(|args| args.repr(root))
                .unwrap_or("".to_string())
        )
    }
}

impl Repr for GenericBound {
    fn repr(&self, root: &CachedItem) -> String {
        match self {
            GenericBound::TraitBound {
                trait_: path,
                generic_params,
                modifier,
            } => {
                if !generic_params.is_empty() {
                    unimplemented!("Unimplemented: Higher-Rank Trait Bounds")
                } else {
                    format!(
                        "{}{}",
                        match modifier {
                            TraitBoundModifier::None => "",
                            TraitBoundModifier::Maybe => "?",
                            TraitBoundModifier::MaybeConst => {
                                unimplemented!("Unimplemented TraitBoundModifier: {:?}", self)
                            }
                        },
                        path.repr(root)
                    )
                }
            }
            GenericBound::Outlives(a) => a.to_string(),
        }
    }
}
