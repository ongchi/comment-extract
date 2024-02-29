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

use std::{
    ffi::OsStr,
    iter::zip,
    path::{Path, PathBuf},
};

use regex::RegexBuilder;
use rustdoc_types::Item;

use crate::doc_traits::RelativeTo;

pub fn caption(item: &Item) -> String {
    let re = RegexBuilder::new(r"(?:^\s*\n*)*(?P<caption>^\w*.*)(?:\n?)$?")
        .multi_line(true)
        .build()
        .unwrap();

    item.docs
        .as_ref()
        .and_then(|docs| {
            re.captures(docs)
                .map(|cap| cap.name("caption").map(|m| m.as_str()).unwrap_or(""))
        })
        .unwrap_or("")
        .to_string()
}

// Remove lines starts with `#` in code blocks
pub fn hide_code_block_lines(docs: &str) -> String {
    let re_code = RegexBuilder::new(r"^```(?<rust_code>(rust(\s*|\s+.*)?)|\s*)?$")
        .build()
        .unwrap();
    let re_show = RegexBuilder::new(r"^[^#].*|^#\[.*").build().unwrap();

    enum CodeBlock {
        Rust,
        Others,
        None,
    }

    let mut filtered_docs: Vec<&str> = Vec::new();
    let mut stat = CodeBlock::None;

    for line in docs.lines() {
        match stat {
            CodeBlock::Rust => {
                if re_show.captures(line).is_some() {
                    filtered_docs.push(line);
                }
                if re_code.captures(line).is_some() {
                    stat = CodeBlock::None;
                }
            }
            CodeBlock::Others => {
                filtered_docs.push(line);
                if re_code.captures(line).is_some() {
                    stat = CodeBlock::None;
                }
            }
            CodeBlock::None => {
                if let Some(cap) = re_code.captures(line) {
                    stat = match cap.name("rust_code") {
                        Some(_) => {
                            // The rustdoc code blocks without specifyinig a language would be `rust`, and
                            // may contain additional attributes.
                            // Replace with this line to work with Sphinix.
                            filtered_docs.push("```rust");
                            CodeBlock::Rust
                        }
                        _ => {
                            filtered_docs.push(line);
                            CodeBlock::Others
                        }
                    };
                } else {
                    filtered_docs.push(line);
                };
            }
        }
    }

    filtered_docs.join("\n")
}

impl<P> RelativeTo<P> for Path
where
    P: AsRef<Path>,
{
    fn relative_to(&self, other: &P) -> PathBuf {
        let left = self.iter().collect::<Vec<&OsStr>>();
        let right = other.as_ref().iter().collect::<Vec<&OsStr>>();

        let mut d = 0;
        for (l, r) in zip(left.iter(), right.iter()) {
            if l == r {
                d += 1
            }
        }

        (0..(left.len() - d))
            .map(|_| OsStr::new(".."))
            .chain(right.into_iter().skip(d))
            .collect()
    }
}
