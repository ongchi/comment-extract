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

mod repr;
mod segment;
mod utils;

use std::fs::read_to_string;

use anyhow::Error;
use clap::Parser;

use segment::SegmentCollections;
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser, PartialEq)]
#[clap(author, version, about, long_about= None)]
struct Args {
    #[clap(long, default_value = "rustdoc-extract.toml")]
    config: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    manifest_path: Option<String>,
    output_path: String,
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Package {
    name: String,
    module_path: Option<String>,
    kind: String,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();
    let config: Config = toml::from_str(&read_to_string(args.config)?)?;
    let collections: SegmentCollections = config.try_into()?;

    collections.extract()
}
