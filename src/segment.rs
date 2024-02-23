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
use regex::RegexBuilder;
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, Item, ItemEnum, ItemKind, Term,
    TraitBoundModifier, Type, TypeBinding, TypeBindingKind, Visibility,
};

use crate::utils::{extract_associated_methods, hide_code_block_lines};
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
    fn _items_to_export(&'a self) -> Result<Vec<ExportItem<'a>>, Error> {
        let mut items = vec![];

        for option in &self.export_options {
            let crate_ = self.packages.get(&option.package).unwrap();
            items.extend(
                crate_
                    .paths
                    .iter()
                    .filter(|(_, summary)| summary.kind == option.kind)
                    .filter(|(_, summary)| {
                        if let Some(selected_path) = &option.module_path {
                            let this_path = summary.path.iter().collect::<PathBuf>();
                            this_path.ancestors().any(|p| p == selected_path)
                        } else {
                            true
                        }
                    })
                    .filter_map(|(id, _)| match crate_.index.get(id) {
                        Some(item) => {
                            if item.visibility == Visibility::Public {
                                let export_item = match option.kind {
                                    ItemKind::Function => ExportItem::Function { crate_, item },
                                    ItemKind::Struct => ExportItem::Struct { crate_, item },
                                    _ => unimplemented!("unsupported item kind: {:?}", option.kind),
                                };
                                Some((id.clone(), export_item))
                            } else {
                                None
                            }
                        }
                        None => None,
                    })
                    .map(|(_, item)| item),
            );
        }

        Ok(items)
    }

    pub fn export(&'a self) -> Result<(), Error> {
        let output_root: PathBuf = self.output_root.clone();

        for item in self._items_to_export()? {
            match item {
                ExportItem::Function { crate_, item } => Segment::new(crate_)
                    .extract(SegmentType::FunctionItem(item))
                    .write_md(&output_root)?,
                ExportItem::Struct { crate_, item } => Segment::new(crate_)
                    .extract(SegmentType::StructItem(item))
                    .write_md(&output_root)?,
            }
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

#[derive(Debug)]
pub enum ExportItem<'a> {
    Function { crate_: &'a Crate, item: &'a Item },
    Struct { crate_: &'a Crate, item: &'a Item },
}

pub struct Segment<'a> {
    crate_: &'a Crate,
    type_: SegmentType<'a>,
}

#[derive(Debug)]
pub enum SegmentType<'a> {
    Empty,
    FunctionItem(&'a Item),
    StructItem(&'a Item),
    ItemEnum(&'a ItemEnum),
    ResolvedPath(&'a rustdoc_types::Path),
    GenericArgs(&'a GenericArgs),
    Type(&'a Type),
    GenericBound(&'a GenericBound),
    TypeBinding(&'a TypeBinding),
}

impl<'a> Segment<'a> {
    pub fn new(crate_: &'a Crate) -> Self {
        Self {
            crate_,
            type_: SegmentType::Empty,
        }
    }

    pub fn extract(&self, type_: SegmentType<'a>) -> Self {
        Self {
            crate_: self.crate_,
            type_,
        }
    }

    pub fn write_md(&self, root: &Path) -> Result<(), std::io::Error> {
        let mut root = PathBuf::from(root);
        if let Some(summary) = self.crate_.paths.get(&self._item().id) {
            for sub_path in summary
                .path
                .split_last()
                .map(|(_, paths)| paths)
                .unwrap_or_default()
            {
                root = root.join(sub_path);
            }
        }
        create_dir_all(&root)?;

        let filename = root.join(format!("{}.md", self.name()));

        let mut file = File::create(&filename)?;
        file.write_all(self.to_string().as_bytes())?;
        if let SegmentType::StructItem(item) = self.type_ {
            for method in extract_associated_methods(self.crate_, item) {
                self.extract(SegmentType::FunctionItem(method))
                    .write_md(&filename.parent().unwrap().join(self.name()))?;
            }
        }
        Ok(())
    }

    fn _item(&self) -> &Item {
        match self.type_ {
            SegmentType::StructItem(s) => s,
            SegmentType::FunctionItem(f) => f,
            _ => panic!("Invalid type: {:?}", self.type_),
        }
    }

    pub fn name(&self) -> &str {
        self._item().name.as_deref().unwrap_or("")
    }

    fn _docs(&self) -> &str {
        self._item().docs.as_deref().unwrap_or("")
    }

    pub fn docs(&self) -> String {
        hide_code_block_lines(self._docs())
    }

    pub fn caption(&self) -> &str {
        let re = RegexBuilder::new(r"(?:^\s*\n*)*(?P<caption>^\w*.*)(?:\n?)$?")
            .multi_line(true)
            .build()
            .unwrap();

        re.captures(self._docs())
            .map(|cap| cap.name("caption").map(|m| m.as_str()).unwrap_or(""))
            .unwrap_or("")
    }
}

impl<'a> std::fmt::Display for Segment<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.type_ {
            SegmentType::Empty => {
                write!(f, "")
            }

            SegmentType::FunctionItem(item) => {
                let name = self.name();
                write!(
                    f,
                    r#"
# {}

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
                    self.extract(SegmentType::ItemEnum(&item.inner)),
                    self.docs()
                )
            }

            SegmentType::StructItem(item) => {
                write!(f, "# {}\n\n{}", self.name(), self.docs(),)?;

                let methods = extract_associated_methods(self.crate_, item)
                    .into_iter()
                    .map(|item| self.extract(SegmentType::FunctionItem(item)))
                    .map(|method| {
                        format!(
                            "| [{}]({}/{}.md) | {} |",
                            method.name(),
                            self.name(),
                            method.name(),
                            method.caption()
                        )
                    })
                    .collect::<Vec<String>>()
                    .join("\n");

                if !methods.is_empty() {
                    write!(
                        f,
                        "\n\n# Methods\n| Method | Description |\n| --- | --- |\n{}",
                        methods
                    )?
                };

                Ok(())
            }

            SegmentType::ItemEnum(item) => match item {
                ItemEnum::Function(func) => {
                    write!(
                        f,
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
                                self.extract(SegmentType::Type(t))
                            ))
                            .collect::<Vec<String>>()
                            .join(", "),
                        func.decl
                            .output
                            .as_ref()
                            .map(|t| format!(" → {}", self.extract(SegmentType::Type(t))))
                            .unwrap_or("".to_string())
                    )
                }

                _ => unimplemented!("Unimplemented item: {:?}", item),
            },

            SegmentType::ResolvedPath(p) => write!(
                f,
                "{}{}",
                p.name,
                p.args
                    .as_ref()
                    .map(|args| self.extract(SegmentType::GenericArgs(args)).to_string())
                    .unwrap_or("".to_string())
            ),

            SegmentType::Type(Type::Primitive(p)) => write!(f, "{}", p),

            SegmentType::Type(Type::ResolvedPath(p)) => {
                write!(f, "{}", self.extract(SegmentType::ResolvedPath(p)))
            }

            SegmentType::Type(Type::DynTrait(dyn_trait)) => write!(
                f,
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
                            self.extract(SegmentType::ResolvedPath(&poly_trait.trait_))
                        )
                    })
                    .chain(dyn_trait.lifetime.iter().map(|t| t.to_string()))
                    .collect::<Vec<String>>()
                    .join(" + ")
            ),

            SegmentType::Type(Type::Generic(t)) => write!(f, "{}", &t),

            SegmentType::Type(Type::BorrowedRef {
                lifetime,
                mutable,
                type_,
            }) => {
                write!(
                    f,
                    "&{}{}{}",
                    lifetime
                        .as_ref()
                        .map(|a| format!("{} ", a))
                        .unwrap_or("".to_string()),
                    if *mutable { "mut " } else { "" },
                    self.extract(SegmentType::Type(type_))
                )
            }

            SegmentType::Type(Type::Tuple(tuple)) => write!(
                f,
                "({})",
                tuple
                    .iter()
                    .map(|t| self.extract(SegmentType::Type(t)).to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),

            SegmentType::Type(Type::Slice(slice)) => {
                write!(f, "[{}]", self.extract(SegmentType::Type(slice)))
            }

            SegmentType::Type(Type::Array { type_, len }) => {
                write!(f, "[{}: {}]", self.extract(SegmentType::Type(type_)), len)
            }

            SegmentType::Type(Type::ImplTrait(bounds)) => {
                write!(
                    f,
                    "impl {}",
                    bounds
                        .iter()
                        .map(|b| self.extract(SegmentType::GenericBound(b)).to_string())
                        .collect::<Vec<String>>()
                        .join(" + ")
                )
            }

            SegmentType::Type(unknown) => unimplemented!("Unimplemented Type: {:?}", unknown),

            SegmentType::GenericArgs(GenericArgs::AngleBracketed { args, bindings }) => {
                if !args.is_empty() || !bindings.is_empty() {
                    write!(
                        f,
                        "&lt;{}&gt;",
                        args.iter()
                            .map(|arg| match arg {
                                GenericArg::Lifetime(a) => a.clone(),
                                GenericArg::Type(t) =>
                                    self.extract(SegmentType::Type(t)).to_string(),
                                unknown =>
                                    unimplemented!("Unimplemented GenericArg: {:?}", unknown),
                            })
                            .chain(bindings.iter().map(|binding| {
                                self.extract(SegmentType::TypeBinding(binding)).to_string()
                            }))
                            .collect::<Vec<String>>()
                            .join(", ")
                    )
                } else {
                    write!(f, "")
                }
            }

            SegmentType::GenericArgs(unknown) => {
                unimplemented!("Unimplemented GenericArgs: {:?}", unknown)
            }

            SegmentType::GenericBound(b) => match b {
                GenericBound::TraitBound {
                    trait_,
                    generic_params,
                    modifier,
                } => {
                    write!(
                        f,
                        "{}{}{}",
                        if !generic_params.is_empty() {
                            unimplemented!("Unimplemented: Higher-Rank Trait Bounds")
                        } else {
                            ""
                        },
                        match modifier {
                            TraitBoundModifier::None => "",
                            TraitBoundModifier::Maybe => "?",
                            TraitBoundModifier::MaybeConst => {
                                unimplemented!("Unimplemented TraitBoundModifier: {:?}", b)
                            }
                        },
                        self.extract(SegmentType::ResolvedPath(trait_))
                    )
                }
                GenericBound::Outlives(a) => write!(f, "{}", a),
            },

            SegmentType::TypeBinding(TypeBinding {
                name,
                args,
                binding,
            }) => write!(
                f,
                "{}{}{}",
                name,
                self.extract(SegmentType::GenericArgs(args)),
                match binding {
                    TypeBindingKind::Equality(term) => {
                        match term {
                            Term::Type(t) => self.extract(SegmentType::Type(t)),
                            Term::Constant(c) => {
                                unimplemented!("Unimplemented TypeBindingKind: {:?}", c)
                            }
                        }
                    }
                    TypeBindingKind::Constraint(c) =>
                        unimplemented!("Unimplemented TypeBindingKing: {:?}", c),
                }
            ),
        }
    }
}
