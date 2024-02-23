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
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Error};
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, Item, ItemEnum, ItemKind, ItemSummary, Term,
    TraitBoundModifier, Type, TypeBinding, TypeBindingKind, Visibility,
};

use crate::utils::{associated_methods, caption, hide_code_block_lines};
use crate::Args;

#[derive(Debug)]
pub struct ExportOption {
    package: String,
    module_path: Option<PathBuf>,
    kind: ItemKind,
}

#[derive(Debug)]
pub struct SegmentCollections {
    packages: HashMap<String, Crate>,
    export_options: Vec<ExportOption>,
    output_root: PathBuf,
}

impl<'a> SegmentCollections {
    fn _items_to_export(&'a self) -> Result<Vec<ItemRef<'a>>, Error> {
        let mut items = vec![];

        for option in &self.export_options {
            let crate_ = self.packages.get(&option.package).unwrap();
            items.extend(
                (crate_.paths.iter())
                    .filter(|(_, summ)| summ.kind == option.kind)
                    .filter(|(_, summ)| {
                        option
                            .module_path
                            .as_ref()
                            .map(|p| summ.path.iter().collect::<PathBuf>().starts_with(p))
                            .unwrap_or(true)
                    })
                    .filter_map(|(id, summ)| crate_.index.get(id).map(|item| (id, item, summ)))
                    .filter(|(_, item, _)| item.visibility == Visibility::Public)
                    .flat_map(|(_, item, item_summary)| {
                        let root = ItemRef::new(crate_, item, Summary::ItemSummary(item_summary));
                        root.associated_methods().into_iter().chain([root])
                    }),
            );
        }

        Ok(items)
    }

    pub fn export(&'a self) -> Result<(), Error> {
        let export_items = self._items_to_export()?;
        for item_ref in &export_items {
            item_ref.write_md(&self.output_root)?;
        }

        Ok(())
    }
}

impl TryFrom<Args> for SegmentCollections {
    type Error = Error;

    fn try_from(value: Args) -> Result<Self, Self::Error> {
        let mut builder = rustdoc_json::Builder::default()
            .manifest_path(&value.manifest_path)
            .toolchain("nightly")
            .all_features(true)
            .clear_target_dir();

        if let Some(pkg) = &value.package {
            builder = builder.package(pkg);
        }

        let json_path = builder.build()?;
        let file = File::open(json_path).map_err(|e| anyhow!(e))?;
        let reader = BufReader::new(file);
        let crate_: Crate = serde_json::from_reader(reader)?;

        let package = match value.package {
            Some(pkg) => pkg,
            None => crate_
                .index
                .get(&crate_.root)
                .and_then(|item| item.name.clone())
                .unwrap(),
        };

        let packages = [(package.clone(), crate_)].into_iter().collect();

        Ok(Self {
            packages,
            export_options: vec![ExportOption {
                package,
                module_path: value.module_path.map(|s| s.split("::").collect()),
                kind: serde_plain::from_str(&value.kind)?,
            }],
            output_root: PathBuf::from(value.output_path),
        })
    }
}

// Save necessory information for associated function of a struct
// which does not have an ItemSummary.
#[derive(Debug)]
enum Summary<'a> {
    ItemSummary(&'a ItemSummary),
    Path(PathBuf),
}

#[derive(Debug)]
pub struct ItemRef<'a> {
    pkg: &'a Crate,
    item: &'a Item,
    summary: Summary<'a>,
}

impl<'a> ItemRef<'a> {
    fn new(pkg: &'a Crate, item: &'a Item, summary: Summary<'a>) -> Self {
        Self { pkg, item, summary }
    }

    fn name(&self) -> &'a str {
        self.item.name.as_deref().unwrap_or("")
    }

    fn kind(&self) -> ItemKind {
        match self.summary {
            Summary::ItemSummary(summ) => summ.kind.clone(),
            Summary::Path(_) => ItemKind::Function,
        }
    }

    fn path(&self) -> PathBuf {
        match &self.summary {
            Summary::ItemSummary(summ) => summ.path.iter().collect(),
            Summary::Path(p) => p.clone(),
        }
    }

    fn docs(&self) -> String {
        hide_code_block_lines(self.item.docs.as_deref().unwrap_or(""))
    }

    fn associated_methods(&self) -> Vec<Self> {
        match &self.item.inner {
            ItemEnum::Struct(s) => s
                .impls
                .iter()
                .filter_map(|id| self.pkg.index.get(id))
                .filter_map(|item| match &item.inner {
                    ItemEnum::Impl(impl_) => match impl_.trait_ {
                        Some(_) => None,
                        None => Some(impl_.items.as_slice()),
                    },
                    _ => None,
                })
                .flatten()
                .flat_map(|id| self.pkg.index.get(id).map(|item| (id, item)))
                .map(|(_, item)| {
                    Self::new(
                        self.pkg,
                        item,
                        Summary::Path(self.path().join(self.item.name.as_deref().unwrap_or(""))),
                    )
                })
                .collect(),
            _ => vec![],
        }
    }

    fn write_md(&self, root: &Path) -> Result<(), Error> {
        let root = PathBuf::from(root).join(self.path());
        let root = root.parent().unwrap();
        create_dir_all(root)?;
        let filename = root.join(format!("{}.md", self.name()));

        let mut file = File::create(filename)?;
        file.write_all(self.repr().as_bytes())?;
        Ok(())
    }
}

trait Repr<'a> {
    fn repr(&self) -> String;
}

impl<'a> Repr<'a> for ItemRef<'a> {
    fn repr(&self) -> String {
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
                    self.item.inner.repr(),
                    self.docs()
                )
            }

            ItemKind::Struct => {
                let methods = associated_methods(self.pkg, self.item)
                    .into_iter()
                    .map(|method| {
                        format!(
                            "| {} | {} |",
                            method.name.as_deref().unwrap(),
                            caption(method)
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
    fn repr(&self) -> String {
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
                            t.repr()
                        ))
                        .collect::<Vec<String>>()
                        .join(", "),
                    func.decl
                        .output
                        .as_ref()
                        .map(|t| format!(" → {}", t.repr()))
                        .unwrap_or("".to_string())
                )
            }
            _ => unimplemented!("Unimplemented ItemEnum: {:?}", self),
        }
    }
}

impl<'a> Repr<'a> for Type {
    fn repr(&self) -> String {
        match self {
            Type::Primitive(p) => p.clone(),

            Type::ResolvedPath(p) => p.repr(),

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
                            &poly_trait.trait_.repr()
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
                    type_.repr()
                )
            }

            Type::Tuple(tuple) => format!(
                "({})",
                tuple
                    .iter()
                    .map(|t| t.repr())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),

            Type::Slice(slice) => format!("[{}]", slice.repr()),

            Type::Array { type_, len } => format!("[{}: {}]", type_.repr(), len),

            Type::ImplTrait(bounds) => {
                format!(
                    "impl {}",
                    bounds
                        .iter()
                        .map(|b| b.repr())
                        .collect::<Vec<String>>()
                        .join(" + ")
                )
            }

            unknown => unimplemented!("Unimplemented Type: {:?}", unknown),
        }
    }
}

impl<'a> Repr<'a> for TypeBinding {
    fn repr(&self) -> String {
        format!(
            "{}{}{}",
            self.name,
            self.args.repr(),
            match &self.binding {
                TypeBindingKind::Equality(term) => {
                    match term {
                        Term::Type(t) => t.repr(),
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
    fn repr(&self) -> String {
        match self {
            GenericArgs::AngleBracketed { args, bindings } => {
                if !args.is_empty() || !bindings.is_empty() {
                    format!(
                        "&lt;{}&gt;",
                        args.iter()
                            .map(|arg| match arg {
                                GenericArg::Lifetime(a) => a.clone(),
                                GenericArg::Type(t) => t.repr(),
                                unknown =>
                                    unimplemented!("Unimplemented GenericArg: {:?}", unknown),
                            })
                            .chain(bindings.iter().map(|binding| { binding.repr() }))
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

impl<'a> Repr<'a> for rustdoc_types::Path {
    fn repr(&self) -> String {
        format!(
            "{}{}",
            self.name,
            self.args
                .as_ref()
                .map(|args| args.repr())
                .unwrap_or("".to_string())
        )
    }
}

impl<'a> Repr<'a> for GenericBound {
    fn repr(&self) -> String {
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
                        trait_.repr()
                    )
                }
            }
            GenericBound::Outlives(a) => a.to_string(),
        }
    }
}
