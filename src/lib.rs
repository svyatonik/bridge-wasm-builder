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

//! # Wasm builder is a utility for building a project as a Wasm binary
//!
//! The Wasm builder is a tool that integrates the process of building the WASM binary of your
//! project into the main `cargo` build process.
//!
//! ## Project setup
//!
//! A project that should be compiled as a Wasm binary needs to:
//!
//! 1. Add a `build.rs` file.
//! 2. Add `wasm-builder` as dependency into `build-dependencies`.
//!
//! The `build.rs` file needs to contain the following code:
//!
//! ```no_run
//! use substrate_wasm_builder::WasmBuilder;
//!
//! fn main() {
//!     WasmBuilder::new()
//!         // Tell the builder to build the project (crate) this `build.rs` is part of.
//!         .with_current_project()
//!         // Make sure to export the `heap_base` global, this is required by Substrate
//!         .export_heap_base()
//!         // Build the Wasm file so that it imports the memory (need to be provided by at instantiation)
//!         .import_memory()
//!         // Build it.
//!         .build()
//! }
//! ```
//!
//! As the final step, you need to add the following to your project:
//!
//! ```ignore
//! include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));
//! ```
//!
//! This will include the generated Wasm binary as two constants `WASM_BINARY` and
//! `WASM_BINARY_BLOATY`. The former is a compact Wasm binary and the latter is the Wasm binary as
//! being generated by the compiler. Both variables have `Option<&'static [u8]>` as type.
//!
//! ### Feature
//!
//! Wasm builder supports to enable cargo features while building the Wasm binary. By default it
//! will enable all features in the wasm build that are enabled for the native build except the
//! `default` and `std` features. Besides that, wasm builder supports the special `runtime-wasm`
//! feature. This `runtime-wasm` feature will be enabled by the wasm builder when it compiles the
//! Wasm binary. If this feature is not present, it will not be enabled.
//!
//! ## Environment variables
//!
//! By using environment variables, you can configure which Wasm binaries are built and how:
//!
//! - `SKIP_WASM_BUILD` - Skips building any Wasm binary. This is useful when only native should be
//!   recompiled. If this is the first run and there doesn't exist a Wasm binary, this will set both
//!   variables to `None`.
//! - `WASM_BUILD_TYPE` - Sets the build type for building Wasm binaries. Supported values are
//!   `release` or `debug`. By default the build type is equal to the build type used by the main
//!   build.
//! - `FORCE_WASM_BUILD` - Can be set to force a Wasm build. On subsequent calls the value of the
//!   variable needs to change. As wasm-builder instructs `cargo` to watch for file changes this
//!   environment variable should only be required in certain circumstances.
//! - `WASM_BUILD_RUSTFLAGS` - Extend `RUSTFLAGS` given to `cargo build` while building the wasm
//!   binary.
//! - `WASM_BUILD_NO_COLOR` - Disable color output of the wasm build.
//! - `WASM_TARGET_DIRECTORY` - Will copy any build Wasm binary to the given directory. The path
//!   needs to be absolute.
//! - `WASM_BUILD_TOOLCHAIN` - The toolchain that should be used to build the Wasm binaries. The
//!   format needs to be the same as used by cargo, e.g. `nightly-2020-02-20`.
//!
//! Each project can be skipped individually by using the environment variable
//! `SKIP_PROJECT_NAME_WASM_BUILD`. Where `PROJECT_NAME` needs to be replaced by the name of the
//! cargo project, e.g. `node-runtime` will be `NODE_RUNTIME`.
//!
//! ## Prerequisites:
//!
//! Wasm builder requires the following prerequisites for building the Wasm binary:
//!
//! - rust nightly + `wasm32-unknown-unknown` toolchain
//!
//! If a specific rust nightly is installed with `rustup`, it is important that the wasm target is
//! installed as well. For example if installing the rust nightly from 20.02.2020 using `rustup
//! install nightly-2020-02-20`, the wasm target needs to be installed as well `rustup target add
//! wasm32-unknown-unknown --toolchain nightly-2020-02-20`.

use std::{
	env, fs,
	io::BufRead,
	path::{Path, PathBuf},
	process::Command,
};

mod builder;
mod maybe_compressed_blob;
mod prerequisites;
mod wasm_project;

pub use builder::{WasmBuilder, WasmBuilderSelectProject};

/// Environment variable that tells us to skip building the wasm binary.
const SKIP_BUILD_ENV: &str = "SKIP_WASM_BUILD";

/// Environment variable to force a certain build type when building the wasm binary.
/// Expects "debug" or "release" as value.
///
/// By default the WASM binary uses the same build type as the main cargo build.
const WASM_BUILD_TYPE_ENV: &str = "WASM_BUILD_TYPE";

/// Environment variable to extend the `RUSTFLAGS` variable given to the wasm build.
const WASM_BUILD_RUSTFLAGS_ENV: &str = "WASM_BUILD_RUSTFLAGS";

/// Environment variable to set the target directory to copy the final wasm binary.
///
/// The directory needs to be an absolute path.
const WASM_TARGET_DIRECTORY: &str = "WASM_TARGET_DIRECTORY";

/// Environment variable to disable color output of the wasm build.
const WASM_BUILD_NO_COLOR: &str = "WASM_BUILD_NO_COLOR";

/// Environment variable to set the toolchain used to compile the wasm binary.
const WASM_BUILD_TOOLCHAIN: &str = "WASM_BUILD_TOOLCHAIN";

/// Environment variable that makes sure the WASM build is triggered.
const FORCE_WASM_BUILD_ENV: &str = "FORCE_WASM_BUILD";

/// Write to the given `file` if the `content` is different.
fn write_file_if_changed(file: impl AsRef<Path>, content: impl AsRef<str>) {
	if fs::read_to_string(file.as_ref()).ok().as_deref() != Some(content.as_ref()) {
		fs::write(file.as_ref(), content.as_ref())
			.unwrap_or_else(|_| panic!("Writing `{}` can not fail!", file.as_ref().display()));
	}
}

/// Copy `src` to `dst` if the `dst` does not exist or is different.
fn copy_file_if_changed(src: PathBuf, dst: PathBuf) {
	let src_file = fs::read_to_string(&src).ok();
	let dst_file = fs::read_to_string(&dst).ok();

	if src_file != dst_file {
		fs::copy(&src, &dst).unwrap_or_else(|_| {
			panic!("Copying `{}` to `{}` can not fail; qed", src.display(), dst.display())
		});
	}
}

/// Get a cargo command that compiles with nightly
fn get_nightly_cargo() -> CargoCommand {
	let env_cargo =
		CargoCommand::new(&env::var("CARGO").expect("`CARGO` env variable is always set by cargo"));
	let default_cargo = CargoCommand::new("cargo");
	let rustup_run_nightly = CargoCommand::new_with_args("rustup", &["run", "nightly", "cargo"]);
	let wasm_toolchain = env::var(WASM_BUILD_TOOLCHAIN).ok();

	// First check if the user requested a specific toolchain
	if let Some(cmd) = wasm_toolchain.and_then(|t| get_rustup_nightly(Some(t))) {
		cmd
	} else if env_cargo.is_nightly() {
		env_cargo
	} else if default_cargo.is_nightly() {
		default_cargo
	} else if rustup_run_nightly.is_nightly() {
		rustup_run_nightly
	} else {
		// If no command before provided us with a nightly compiler, we try to search one
		// with rustup. If that fails as well, we return the default cargo and let the prequisities
		// check fail.
		get_rustup_nightly(None).unwrap_or(default_cargo)
	}
}

/// Get a nightly from rustup. If `selected` is `Some(_)`, a `CargoCommand` using the given
/// nightly is returned.
fn get_rustup_nightly(selected: Option<String>) -> Option<CargoCommand> {
	let host = format!("-{}", env::var("HOST").expect("`HOST` is always set by cargo"));

	let version = match selected {
		Some(selected) => selected,
		None => {
			let output = Command::new("rustup").args(&["toolchain", "list"]).output().ok()?.stdout;
			let lines = output.as_slice().lines();

			let mut latest_nightly = None;
			for line in lines.filter_map(|l| l.ok()) {
				if line.starts_with("nightly-") && line.ends_with(&host) {
					// Rustup prints them sorted
					latest_nightly = Some(line.clone());
				}
			}

			latest_nightly?.trim_end_matches(&host).into()
		},
	};

	Some(CargoCommand::new_with_args("rustup", &["run", &version, "cargo"]))
}

/// Wraps a specific command which represents a cargo invocation.
#[derive(Debug)]
struct CargoCommand {
	program: String,
	args: Vec<String>,
}

impl CargoCommand {
	fn new(program: &str) -> Self {
		CargoCommand { program: program.into(), args: Vec::new() }
	}

	fn new_with_args(program: &str, args: &[&str]) -> Self {
		CargoCommand {
			program: program.into(),
			args: args.iter().map(ToString::to_string).collect(),
		}
	}

	fn command(&self) -> Command {
		let mut cmd = Command::new(&self.program);
		cmd.args(&self.args);
		cmd
	}

	/// Check if the supplied cargo command is a nightly version
	fn is_nightly(&self) -> bool {
		// `RUSTC_BOOTSTRAP` tells a stable compiler to behave like a nightly. So, when this env
		// variable is set, we can assume that whatever rust compiler we have, it is a nightly
		// compiler. For "more" information, see:
		// https://github.com/rust-lang/rust/blob/fa0f7d0080d8e7e9eb20aa9cbf8013f96c81287f/src/libsyntax/feature_gate/check.rs#L891
		env::var("RUSTC_BOOTSTRAP").is_ok() ||
			self.command()
				.arg("--version")
				.output()
				.map_err(|_| ())
				.and_then(|o| String::from_utf8(o.stdout).map_err(|_| ()))
				.unwrap_or_default()
				.contains("-nightly")
	}
}

/// Wraps a [`CargoCommand`] and the version of `rustc` the cargo command uses.
struct CargoCommandVersioned {
	command: CargoCommand,
	version: String,
}

impl CargoCommandVersioned {
	fn new(command: CargoCommand, version: String) -> Self {
		Self { command, version }
	}

	/// Returns the `rustc` version.
	fn rustc_version(&self) -> &str {
		&self.version
	}
}

impl std::ops::Deref for CargoCommandVersioned {
	type Target = CargoCommand;

	fn deref(&self) -> &CargoCommand {
		&self.command
	}
}

/// Returns `true` when color output is enabled.
fn color_output_enabled() -> bool {
	env::var(crate::WASM_BUILD_NO_COLOR).is_err()
}
