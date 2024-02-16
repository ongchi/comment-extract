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

#![feature(box_patterns)]

use std::fs::{create_dir_all, File};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Error};
use clap::Parser;
use once_cell::sync::OnceCell;
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, Item, ItemEnum, ItemKind, Term,
    TraitBoundModifier, Type, TypeBinding, TypeBindingKind, Visibility,
};

static CRATE: OnceCell<Crate> = OnceCell::new();

#[derive(Debug, Parser, PartialEq)]
#[clap(author, version, about, long_about= None)]
struct Args {
    #[clap(long, default_value = "Cargo.toml", help = "Path to Cargo.toml")]
    manifest_path: String,

    #[clap(long, help = "Package to extract")]
    package: Option<String>,

    #[clap(long, help = "Filter by module path")]
    module_path: Option<String>,

    #[clap(long, default_value = "function", help = "Filter by item kind")]
    kind: String,

    #[clap(long, help = "The path to generated outputs")]
    output_path: String,
}

macro_rules! unwrap_or_empty {
    ($item:expr) => {
        $item.as_ref().map(|s| s.as_str()).unwrap_or("")
    };
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    let mut builder = rustdoc_json::Builder::default()
        .manifest_path(Path::new(&args.manifest_path))
        .toolchain("nightly")
        .all_features(true);

    if let Some(ref pkg) = args.package {
        builder = builder.package(pkg);
    }

    // Crate information from rustdoc JSON
    CRATE.get_or_try_init(|| {
        let json_path = builder.build()?;
        let file = File::open(&json_path).map_err(|e| anyhow!(e))?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|e| anyhow!(e))
    })?;
    let crate_ = CRATE.get().unwrap();

    let selected_kind: ItemKind = serde_plain::from_str(&args.kind)?;

    let items = crate_
        .paths
        .iter()
        .filter(|(_, item)| item.kind == selected_kind)
        .filter(|(_, item)| {
            if let Some(ref module) = args.module_path {
                item.path.join("::").starts_with(module)
            } else {
                true
            }
        })
        .filter_map(|(idx, _)| crate_.index.get(idx))
        .filter(|item| item.visibility == Visibility::Public)
        .collect::<Vec<_>>();

    let output_path = PathBuf::from(args.output_path);

    match selected_kind {
        ItemKind::Function => {
            for i in items {
                write_segment(Segment::FunctionItem(i), &output_path)?;
            }
        }
        ItemKind::Struct => {
            for i in items {
                write_segment(Segment::StructItem(i), &output_path)?;
            }
        }
        _ => {
            unimplemented!("Unimplemented ItemKind: {:?}", selected_kind)
        }
    }

    Ok(())
}

fn write_segment(segment: Segment, root: &Path) -> Result<(), std::io::Error> {
    let filename = md_filename(root, segment.as_ref())?;
    let mut file = File::create(&filename)?;
    file.write_all(segment.to_string().as_bytes())?;
    match segment {
        Segment::StructItem(s) => {
            for method in get_struct_methods(&s.inner) {
                write_segment(
                    Segment::FunctionItem(method),
                    &filename
                        .parent()
                        .unwrap()
                        .join(&unwrap_or_empty!(segment.as_ref().name)),
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

impl<'a> AsRef<Item> for Segment<'a> {
    fn as_ref(&self) -> &Item {
        match self {
            Self::FunctionItem(item) => item,
            Self::StructItem(item) => item,
            _ => unimplemented!(),
        }
    }
}

fn md_filename(root: &Path, item: &Item) -> Result<PathBuf, std::io::Error> {
    let mut root_path = PathBuf::from(root);
    if let Some(summary) = CRATE.get().unwrap().paths.get(&item.id) {
        for sub_path in summary
            .path
            .split_last()
            .map(|(_, paths)| paths)
            .unwrap_or_default()
        {
            root_path = root_path.join(sub_path);
        }
    }
    create_dir_all(&root_path)?;
    Ok(root_path.join(format!("{}.md", item.name.as_ref().unwrap())))
}

fn get_struct_methods(struct_: &ItemEnum) -> Vec<&Item> {
    let crate_ = CRATE.get().unwrap();

    match struct_ {
        ItemEnum::Struct(s) => s
            .impls
            .iter()
            .filter_map(|id| crate_.index.get(id))
            .filter_map(|item| match item.inner {
                ItemEnum::Impl(ref impl_) => match impl_.trait_ {
                    Some(_) => None,
                    None => Some(
                        impl_
                            .items
                            .iter()
                            .map(|id| crate_.index.get(id).unwrap())
                            .collect::<Vec<&Item>>(),
                    ),
                },
                _ => None,
            })
            .flatten()
            .collect(),
        _ => unreachable!(),
    }
}

enum Segment<'a> {
    FunctionItem(&'a Item),
    StructItem(&'a Item),
    ItemEnum(&'a ItemEnum),
    ResolvedPath(&'a rustdoc_types::Path),
    GenericArgs(&'a GenericArgs),
    Type(&'a Type),
    GenericBound(&'a GenericBound),
    TypeBinding(&'a TypeBinding),
}

impl<'a> ToString for Segment<'a> {
    fn to_string(&self) -> String {
        match self {
            Self::FunctionItem(item) => {
                let name = unwrap_or_empty!(item.name);
                format!(
                    "# {}\n\nfn {}{}\n\n{}\n",
                    name,
                    name,
                    Segment::ItemEnum(&item.inner).to_string(),
                    unwrap_or_empty!(item.docs)
                )
            }

            Self::StructItem(item) => {
                let name = unwrap_or_empty!(item.name);
                format!("# {}\n\n{}\n", name, unwrap_or_empty!(item.docs))
            }

            Self::ItemEnum(item) => match item {
                ItemEnum::Function(f) => {
                    format!(
                        "({}) -> {}",
                        f.decl
                            .inputs
                            .iter()
                            .map(|(n, t)| format!("{}: {}", n, Segment::Type(t).to_string()))
                            .collect::<Vec<String>>()
                            .join(", "),
                        f.decl
                            .output
                            .as_ref()
                            .map(|t| Segment::Type(t).to_string())
                            .unwrap_or("".to_string())
                    )
                }

                _ => unimplemented!("Unimplemented item: {:?}", item),
            },

            Self::ResolvedPath(p) => format!(
                "{}{}",
                p.name,
                p.args
                    .as_ref()
                    .map(|args| Segment::GenericArgs(args).to_string())
                    .unwrap_or("".to_string())
            ),

            Self::Type(Type::Primitive(p)) => p.to_string(),

            Self::Type(Type::ResolvedPath(p)) => Segment::ResolvedPath(p).to_string(),

            Self::Type(Type::DynTrait(dyn_trait)) => format!(
                "dyn {}",
                dyn_trait
                    .traits
                    .iter()
                    .map(|poly_trait| {
                        format!(
                            "{}{}",
                            if poly_trait.generic_params.len() > 0 {
                                unimplemented!("Unimplemented: Higher-Rank Trait Bounds")
                            } else {
                                ""
                            },
                            Self::ResolvedPath(&poly_trait.trait_).to_string()
                        )
                    })
                    .chain(dyn_trait.lifetime.iter().map(|t| t.to_string()))
                    .collect::<Vec<String>>()
                    .join(" + ")
            ),

            Self::Type(Type::Generic(t)) => t.to_string(),

            Self::Type(Type::BorrowedRef {
                lifetime,
                mutable,
                type_,
            }) => {
                format!(
                    "&{}{}{}",
                    lifetime
                        .as_ref()
                        .map(|a| format!("{} ", a))
                        .unwrap_or("".to_string()),
                    if *mutable { "mut " } else { "" },
                    Segment::Type(type_).to_string()
                )
            }

            Self::Type(Type::Tuple(tuple)) => format!(
                "({})",
                tuple
                    .iter()
                    .map(|t| Segment::Type(t).to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),

            Self::Type(Type::Slice(slice)) => format!("[{}]", Segment::Type(slice).to_string()),

            Self::Type(Type::Array { type_, len }) => {
                format!("[{}: {}]", Segment::Type(type_).to_string(), len)
            }

            Self::Type(Type::ImplTrait(bounds)) => {
                format!(
                    "impl {}",
                    bounds
                        .iter()
                        .map(|b| Segment::GenericBound(b).to_string())
                        .collect::<Vec<String>>()
                        .join(" + ")
                )
            }

            Self::Type(unknown @ _) => unimplemented!("Unimplemented Type: {:?}", unknown),

            Self::GenericArgs(GenericArgs::AngleBracketed { args, bindings }) => {
                if args.len() > 0 || bindings.len() > 0 {
                    format!(
                        "<{}>",
                        args.iter()
                            .map(|arg| match arg {
                                GenericArg::Lifetime(a) => a.clone(),
                                GenericArg::Type(t) => Segment::Type(t).to_string(),
                                unknown @ _ =>
                                    unimplemented!("Unimplemented GenericArg: {:?}", unknown),
                            })
                            .chain(
                                bindings
                                    .iter()
                                    .map(|binding| Segment::TypeBinding(binding).to_string())
                            )
                            .collect::<Vec<String>>()
                            .join(", ")
                    )
                } else {
                    "".to_string()
                }
            }

            Self::GenericArgs(unknown @ _) => {
                unimplemented!("Unimplemented GenericArgs: {:?}", unknown)
            }

            Self::GenericBound(b) => match b {
                GenericBound::TraitBound {
                    trait_,
                    generic_params,
                    modifier,
                } => {
                    format!(
                        "{}{}{}",
                        if generic_params.len() > 0 {
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
                        Self::ResolvedPath(trait_).to_string()
                    )
                }
                GenericBound::Outlives(a) => a.to_string(),
            },

            Self::TypeBinding(TypeBinding {
                name,
                args,
                binding,
            }) => format!(
                "{}{}{}",
                name,
                Segment::GenericArgs(args).to_string(),
                match binding {
                    TypeBindingKind::Equality(term) => {
                        match term {
                            Term::Type(t) => Segment::Type(t).to_string(),
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
