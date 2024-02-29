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

use crate::segment::CachedItem;

pub(crate) trait Repr {
    fn repr(&self, root: &CachedItem) -> String;
}

pub(crate) trait Name {
    fn name(&self) -> &str;
}

pub(crate) trait ModulePath {
    fn path(&self) -> PathBuf;
}

pub(crate) trait RelativeTo<T> {
    fn relative_to(&self, other: &T) -> PathBuf;
}

pub(crate) trait CrossRef<T> {
    fn cross_ref(&self, _to: &T) -> String;

    fn cross_ref_md(&self, _to: &T) -> String;
}

impl<T> CrossRef<T> for &T
where
    T: Name + ModulePath + RelativeTo<T>,
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
