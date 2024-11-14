/*
Copyright 2024 NetApp, Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    https://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

#![allow(clippy::new_without_default)]
// TODO: re-enable dead code checks
#![allow(dead_code)]
extern crate core;

pub mod compressor;
pub mod data;
pub mod frame;
pub mod header;
pub mod utils;

mod csv;
pub mod optimizer;
