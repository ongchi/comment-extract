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

use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use regex::RegexBuilder;
use rustdoc_types::{
    Crate, GenericArg, GenericArgs, GenericBound, Id, Item, ItemEnum, Term, TraitBoundModifier,
    Type, TypeBinding, TypeBindingKind,
};

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
            crate_: &self.crate_,
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
        match self.type_ {
            SegmentType::StructItem(_) => {
                for mid in self.method_ids() {
                    let method = self.crate_.index.get(&mid).unwrap();
                    self.extract(SegmentType::FunctionItem(method))
                        .write_md(&filename.parent().unwrap().join(self.name()))?;
                }
            }
            _ => {}
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
        self._item().name.as_ref().map(|s| s.as_str()).unwrap_or("")
    }

    fn _docs(&self) -> &str {
        self._item().docs.as_ref().map(|s| s.as_str()).unwrap_or("")
    }

    pub fn docs(&self) -> String {
        hide_code_block_lines(self._docs())
    }

    pub fn caption(&self) -> &str {
        let re = RegexBuilder::new(r"(?:^\s*\n*)*(?P<caption>^\w*.*)(?:\n?)$?")
            .multi_line(true)
            .build()
            .unwrap();

        re.captures(&self._docs())
            .map(|cap| cap.name("caption").map(|m| m.as_str()).unwrap_or(""))
            .unwrap_or("")
    }

    fn method_ids(&self) -> Vec<Id> {
        match self.type_ {
            SegmentType::StructItem(item) => match &item.inner {
                ItemEnum::Struct(s) => s
                    .impls
                    .iter()
                    .filter_map(|id| self.crate_.index.get(id))
                    .filter_map(|item| match &item.inner {
                        ItemEnum::Impl(impl_) => match impl_.trait_ {
                            Some(_) => None,
                            None => Some(impl_.items.clone()),
                        },
                        _ => None,
                    })
                    .flatten()
                    .collect(),
                _ => panic!("Not a struct: {:?}", self.type_),
            },
            _ => panic!("Not a struct: {:?}", self.type_),
        }
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

            SegmentType::StructItem(_) => {
                write!(f, "# {}\n\n{}", self.name(), self.docs(),)?;

                let methods = self
                    .method_ids()
                    .iter()
                    .map(|id| self.crate_.index.get(id).unwrap())
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
                            if poly_trait.generic_params.len() > 0 {
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

            SegmentType::Type(unknown @ _) => unimplemented!("Unimplemented Type: {:?}", unknown),

            SegmentType::GenericArgs(GenericArgs::AngleBracketed { args, bindings }) => {
                if args.len() > 0 || bindings.len() > 0 {
                    write!(
                        f,
                        "&lt;{}&gt;",
                        args.iter()
                            .map(|arg| match arg {
                                GenericArg::Lifetime(a) => a.clone(),
                                GenericArg::Type(t) =>
                                    self.extract(SegmentType::Type(t)).to_string(),
                                unknown @ _ =>
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

            SegmentType::GenericArgs(unknown @ _) => {
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

fn hide_code_block_lines(docs: &str) -> String {
    let re_code = RegexBuilder::new(r"^```(?<rust_code>(rust(\s*|\s+.*)?)|\s*)?$")
        .build()
        .unwrap();
    let re_show = RegexBuilder::new(r"^[^#].*|^#\[.*").build().unwrap();

    enum Status {
        InRustCodeBlock,
        InCodeBlock,
        NotInCodeBlock,
    }

    let mut filtered_docs: Vec<&str> = Vec::new();
    let mut stat = Status::NotInCodeBlock;

    for line in docs.lines() {
        match stat {
            Status::InRustCodeBlock => {
                if let Some(_) = re_show.captures(line) {
                    filtered_docs.push(line);
                }
                if let Some(_) = re_code.captures(line) {
                    stat = Status::NotInCodeBlock;
                }
            }
            Status::InCodeBlock => {
                filtered_docs.push(line);
                if let Some(_) = re_code.captures(line) {
                    stat = Status::NotInCodeBlock;
                }
            }
            Status::NotInCodeBlock => {
                filtered_docs.push(line);
                if let Some(cap) = re_code.captures(line) {
                    stat = match cap.name("rust_code") {
                        Some(_) => Status::InRustCodeBlock,
                        _ => Status::InCodeBlock,
                    };
                };
            }
        }
    }

    filtered_docs.join("\n")
}
