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

use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{anyhow, Error};
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, Id, Item, ItemEnum, ItemKind, ItemSummary, Term,
    TraitBoundModifier, Type, TypeBinding, TypeBindingKind,
};

use crate::doc_traits;
use crate::doc_traits::{CrossRef, ExternalLink, ModulePath, Name, RelativeTo, Repr};
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
    items: Vec<CachedItem>,
}

impl<'a> SegmentCollections {
    pub fn extract(&'a self) -> Result<(), Error> {
        for item in &self.items {
            let root = &self.output_root;
            let root = PathBuf::from(root).join(item.path());
            create_dir_all(&root)?;
            let filename = root.join(format!("{}.md", item.name()));

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

        let pool = Rc::new(packages);

        // Collect items to be extract
        let mut items = vec![];
        for option in extract_options {
            let crate_ = pool.get(&option.package.name).unwrap();
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
                        let id = ItemId::new(option.package.name.clone(), id.clone());
                        let item = CachedItem::new(pool.clone(), id);
                        let methods = item.associated_methods();
                        methods.into_iter().chain([item])
                    }),
            )
        }

        Ok(Self { output_root, items })
    }
}

#[derive(Debug, Clone)]
pub struct ItemId {
    pub pkg: String,
    pub id: Id,
}

impl ItemId {
    pub fn new(pkg: String, id: Id) -> Self {
        Self { pkg, id }
    }
}

#[derive(Debug, Clone)]
pub struct CachedItem {
    pub pool: Rc<HashMap<String, Crate>>,
    pub id: ItemId,
    pub item: Item,
    pub item_summary: Option<ItemSummary>,
    pub path: PathBuf,
}

impl CachedItem {
    pub fn new(pool: Rc<HashMap<String, Crate>>, id: ItemId) -> Self {
        let item_summary = pool.get(&id.pkg).unwrap().paths.get(&id.id);
        let path = item_summary
            .map(|summ| summ.path.iter().rev().skip(1).rev().collect::<PathBuf>())
            .unwrap();

        Self::new_with_path(pool, id, path)
    }

    pub fn new_with_path(pool: Rc<HashMap<String, Crate>>, id: ItemId, path: PathBuf) -> Self {
        let item = pool.get(&id.pkg).unwrap().index.get(&id.id).unwrap();
        let item_summary = pool.get(&id.pkg).unwrap().paths.get(&id.id);

        // Save path information for associated function of a struct
        // which does not have an ItemSummary.
        let path = item_summary
            .map(|summ| summ.path.iter().rev().skip(1).rev().collect::<PathBuf>())
            .unwrap_or(path);

        Self {
            pool: pool.clone(),
            id: id.clone(),
            item: item.clone(),
            item_summary: item_summary.cloned(),
            path,
        }
    }

    fn associated_methods(&self) -> Vec<CachedItem> {
        let crate_ = self.pool.get(&self.id.pkg).unwrap();

        match &self.item.inner {
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
                    CachedItem::new_with_path(
                        self.pool.clone(),
                        ItemId::new(self.id.pkg.clone(), id.clone()),
                        (self.path.join(self.item.name.as_ref().unwrap())).clone(),
                    )
                })
                .collect(),
            _ => vec![],
        }
    }

    pub fn crate_(&self) -> &Crate {
        self.pool.get(&self.id.pkg).unwrap()
    }

    fn kind(&self) -> &ItemKind {
        match &self.item_summary {
            Some(summ) => &summ.kind,
            None => &ItemKind::Function,
        }
    }

    fn docs(&self) -> String {
        hide_code_block_lines(self.item.docs.as_deref().unwrap_or(""))
    }
}

impl Name for CachedItem {
    fn name(&self) -> &str {
        self.item.name.as_deref().unwrap_or("")
    }
}

impl ModulePath for CachedItem {
    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}

impl<'a, T> RelativeTo<'a, T> for CachedItem
where
    T: ModulePath,
{
    fn relative_to(&'a self, other: &T) -> PathBuf {
        self.path().relative_to(&other.path())
    }
}

impl<'a> Repr<'a> for CachedItem {
    fn repr(&self, _root: &'a CachedItem) -> String {
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
                    &self.item.inner.repr(self),
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
                            caption(&method.item)
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

impl<'a> Repr<'a> for ItemEnum {
    fn repr(&self, root: &'a CachedItem) -> String {
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
                        .map(|(n, t)| format!(
                            r#"<em class="sig-param n">
    <span class="pre">{}</span>: <span class="pre">{}</span>
</em>"#,
                            n,
                            t.repr(root)
                        ))
                        .collect::<Vec<String>>()
                        .join(", "),
                    func.decl
                        .output
                        .as_ref()
                        .map(|t| format!(" → {}", t.repr(root)))
                        .unwrap_or("".to_string())
                )
            }
            _ => unimplemented!("Unimplemented ItemEnum: {:?}", self),
        }
    }
}

impl ExternalLink for Type {
    fn external_link(&self, root: &CachedItem) -> String {
        match self {
            Type::Primitive(p) => format!("https://doc.rust-lang.org/std/primitive.{}.html", p),

            Type::ResolvedPath(p) => {
                let crate_ = root.crate_();
                let crate_id = crate_.index.get(&p.id).map(|item| item.crate_id).unwrap();

                if crate_id == 0 {
                    let path = crate_
                        .paths
                        .get(&p.id)
                        .map(|s| s.path.join("/"))
                        .unwrap_or("".to_string());
                    format!(
                        "https://docs.rs/{}/{}/{}",
                        root.id.pkg,
                        crate_.crate_version.as_deref().unwrap(),
                        path
                    )
                } else {
                    let root_url = crate_
                        .external_crates
                        .get(&crate_id)
                        .and_then(|c| c.html_root_url.as_deref());
                    match root_url {
                        Some(url) => url.to_string(),
                        None => "".to_string(),
                    }
                }
            }

            _ => unimplemented!(),
        }
    }
}

impl<'a> Repr<'a> for Type {
    fn repr(&self, root: &'a CachedItem) -> String {
        match self {
            Type::Primitive(p) => format!("<a href=\"{}\">{}</a>", self.external_link(root), p),

            Type::ResolvedPath(p) => p.repr(root),

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

            Type::Tuple(tuple) => format!(
                "({})",
                tuple
                    .iter()
                    .map(|tpe| tpe.repr(root))
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

impl<'a> Repr<'a> for TypeBinding {
    fn repr(&self, root: &'a CachedItem) -> String {
        format!(
            "{}{}{}",
            self.name,
            &self.args.repr(root),
            match &self.binding {
                TypeBindingKind::Equality(term) => {
                    match term {
                        Term::Type(t) => t.repr(root),
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

impl<'a> Repr<'a> for GenericArgs {
    fn repr(&self, root: &'a CachedItem) -> String {
        match self {
            GenericArgs::AngleBracketed { args, bindings } => {
                if !args.is_empty() || !bindings.is_empty() {
                    format!(
                        "&lt;{}&gt;",
                        args.iter()
                            .map(|arg| match arg {
                                GenericArg::Lifetime(a) => a.clone(),
                                GenericArg::Type(t) => t.repr(root),
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

impl doc_traits::ItemId for rustdoc_types::Path {
    fn id(&self) -> &Id {
        &self.id
    }
}

impl<'a> Repr<'a> for rustdoc_types::Path {
    fn repr(&self, root: &'a CachedItem) -> String {
        format!(
            "<a href=\"{}\">{}</a>{}",
            self.external_link(root),
            self.name,
            self.args
                .as_deref()
                .map(|args| args.repr(root))
                .unwrap_or("".to_string())
        )
    }
}

impl<'a> Repr<'a> for GenericBound {
    fn repr(&self, root: &'a CachedItem) -> String {
        match self {
            GenericBound::TraitBound {
                trait_,
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
                        trait_.repr(root)
                    )
                }
            }
            GenericBound::Outlives(a) => a.to_string(),
        }
    }
}
