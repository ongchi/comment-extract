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

mod segment;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{anyhow, Error};
use clap::Parser;
use rustdoc_types::{Crate, ItemKind, Visibility};

use segment::{Segment, SegmentType};

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

fn main() -> Result<(), Error> {
    let args = Args::parse();

    let mut builder = rustdoc_json::Builder::default()
        .manifest_path(&args.manifest_path)
        .toolchain("nightly")
        .all_features(true)
        .clear_target_dir();

    if let Some(ref pkg) = args.package {
        builder = builder.package(pkg);
    }

    // Crate information from rustdoc JSON
    let json_path = builder.build()?;
    let file = File::open(&json_path).map_err(|e| anyhow!(e))?;
    let reader = BufReader::new(file);
    let crate_: Crate = serde_json::from_reader(reader).map_err(|e| anyhow!(e))?;
    let segment = Segment::new(&crate_);

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
                segment
                    .extract(SegmentType::FunctionItem(i))
                    .write_md(&output_path)?;
            }
        }
        ItemKind::Struct => {
            for i in items {
                segment
                    .extract(SegmentType::StructItem(i))
                    .write_md(&output_path)?;
            }
        }
        _ => {
            unimplemented!("Unimplemented ItemKind: {:?}", selected_kind)
        }
    }

    Ok(())
}
