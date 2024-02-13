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

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use clap::Parser;
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, ItemEnum, ItemKind, Term, TraitBoundModifier,
    Type, TypeBinding, TypeBindingKind, Visibility,
};

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
}

macro_rules! unwrap_or_empty {
    ($item:expr) => {
        $item.as_ref().map(|s| s.as_str()).unwrap_or("")
    };
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut builder = rustdoc_json::Builder::default()
        .manifest_path(Path::new(&args.manifest_path))
        .toolchain("nightly")
        .all_features(true);

    if let Some(ref pkg) = args.package {
        builder = builder.package(pkg);
    }

    let json_path = builder.build()?;

    // Extract information from rustdoc JSON
    let file = File::open(json_path)?;
    let reader = BufReader::new(file);
    let crate_: Crate = serde_json::from_reader(reader)?;

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

    for i in items {
        let name = unwrap_or_empty!(i.name);
        println!("# {}\n", name);
        println!("fn {}{}\n", name, Segment::ItemEnum(&i.inner).to_string());
        println!("{}\n", unwrap_or_empty!(i.docs));
        println!();
    }

    Ok(())
}

enum Segment<'a> {
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
            Self::ItemEnum(item) => match item {
                ItemEnum::Function(f) => {
                    let _ = 1;
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
