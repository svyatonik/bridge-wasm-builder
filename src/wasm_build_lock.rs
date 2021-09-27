// This file is part of Substrate.

// Copyright (C) 2019-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fs::File;
use std::path::PathBuf;

const RETRY_ATTEMPTS: u32 = 64;

pub struct WasmBuildLock(fd_lock::RwLock<File>);

impl WasmBuildLock {
	pub fn new() -> Self {
		let mut lock_file_path = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("`OUT_DIR` is set by cargo!"));
		loop {
			let file_name = lock_file_path.file_name();
			match file_name.as_ref().and_then(|x| x.to_str()) {
				Some("target") => break,
				Some(_) => {},
				None => panic!("Failed to find `target` directory in OUT_DIR"),
			}
		}
		lock_file_path.push("wasm_build_lock");

		let mut retry_attempts = RETRY_ATTEMPTS;
		loop {
			let locked_file = File::create(&lock_file_path);
			match locked_file {
				Ok(locked_file) => {
					return WasmBuildLock(fd_lock::RwLock::new(locked_file));
				},
				Err(e) if retry_attempts == 0 => {
					panic!("Failed to create `wasm_build_lock` file: {:?}", e);
				}
				Err(e) => {
					println!("Failed to create `wasm_build_lock` file: {:?}", e);
					retry_attempts = retry_attempts - 1;
				}
			}
		}
	}

	pub fn lock<'a>(&'a mut self) -> fd_lock::RwLockWriteGuard<'a, File> {
		self.0.write().expect("Failed to lock `wasm_build_lock` file")
	}
}
