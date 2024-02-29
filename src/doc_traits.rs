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

use std::path::PathBuf;

use rustdoc_types::{Id, ItemEnum};

use crate::segment::CachedItem;

pub(crate) trait ItemId {
    fn id(&self) -> &Id;
}

pub(crate) trait Repr<'a> {
    fn repr(&self, root: &'a CachedItem) -> String;
}

pub(crate) trait Name {
    fn name(&self) -> &str;
}

pub(crate) trait ModulePath {
    fn path(&self) -> PathBuf;
}

pub(crate) trait RelativeTo<'a, T> {
    fn relative_to(&'a self, other: &T) -> PathBuf;
}

pub(crate) trait CrossRef<T> {
    fn cross_ref(&self, _to: &T) -> String;

    fn cross_ref_md(&self, _to: &T) -> String;
}

impl<'a, T> CrossRef<T> for &'a T
where
    T: Name + ModulePath + RelativeTo<'a, T>,
{
    fn cross_ref(&self, to: &T) -> String {
        self.relative_to(to)
            .join(format!("{}.md", to.name()))
            .to_str()
            .unwrap()
            .to_string()
    }

    fn cross_ref_md(&self, to: &T) -> String {
        format!("[{}]({})", to.name(), self.cross_ref(to))
    }
}

pub(crate) trait ExternalLink {
    fn external_link(&self, root: &CachedItem) -> String;
}

impl<T> ExternalLink for T
where
    T: ItemId + std::fmt::Debug,
{
    fn external_link(&self, root: &CachedItem) -> String {
        let crate_ = root.pool.get(&root.id.pkg).unwrap();

        if let Some(item) = crate_.index.get(self.id()) {
            match item.crate_id {
                0 => {
                    let path = crate_
                        .paths
                        .get(&item.id)
                        .map(|s| {
                            s.path
                                .iter()
                                .rev()
                                .skip(1)
                                .rev()
                                .map(|s| s.as_ref())
                                .collect::<Vec<&str>>()
                                .join("/")
                        })
                        .unwrap();

                    format!(
                        "https://docs.rs/{}/{}/{}/{}.{}.html",
                        root.id.pkg,
                        crate_.crate_version.as_deref().unwrap(),
                        path,
                        match &item.inner {
                            ItemEnum::Struct(_) => "struct",
                            ItemEnum::Trait(_) => "trait",
                            others => unimplemented!("Unimplemented type: {:?}", others),
                        },
                        item.name.as_deref().unwrap()
                    )
                }
                _ => {
                    let root_url = crate_
                        .external_crates
                        .get(&item.crate_id)
                        .and_then(|c| c.html_root_url.as_deref());
                    match root_url {
                        Some(url) => url.to_string(),
                        None => "".to_string(),
                    }
                }
            }
        } else {
            "".to_string()
        }
    }
}
