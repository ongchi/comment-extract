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

use rustdoc_types::{
    GenericArg, GenericArgs, GenericBound, ItemEnum, ItemKind, Term, TraitBoundModifier, Type,
    TypeBinding, TypeBindingKind,
};

use crate::{
    segment::{CachedItem, ItemId},
    utils::caption,
};

pub(crate) trait Repr {
    fn repr(&self, root: &CachedItem) -> String;
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
                            "| [{}]({}) | {} |",
                            method.name(),
                            self.cross_ref(&method),
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

            Type::Slice(slice) => format!("&[{}]", slice.repr(root)),

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
