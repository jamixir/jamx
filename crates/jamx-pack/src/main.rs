//! jamx-pack: ELF (riscv64emac-unknown-none-polkavm) -> .polkavm -> JAM service blob (.jam).
//!
//! The published `jam-pvm-build` only goes up to GP 0.7.2; this small tool does the
//! same job for GP 0.8.0 using the published `polkavm-linker` and `jam-program-blob-common`
//! libraries through their public API.

use codec::Encode;
use jam_program_blob_common::{ConventionalMetadata, CrateInfo, ProgramBlob};
use std::{borrow::Cow, env, fs, process::exit};

fn usage() -> ! {
	eprintln!(
		"usage: jamx-pack <input.elf> <output.jam> [--name N] [--version V] [--license L] [--author A]... \\\n       [--min-stack-size BYTES] [--dispatch a,b] [--keep-polkavm path] [--isa jamv1|latest]"
	);
	exit(2)
}

fn main() {
	let args: Vec<String> = env::args().skip(1).collect();
	if args.len() < 2 {
		usage();
	}
	let (input, output) = (&args[0], &args[1]);
	let mut name = String::from("service");
	let mut version = String::from("0.0.0");
	let mut license = String::from("Apache-2.0");
	let mut authors: Vec<String> = Vec::new();
	let mut min_stack: Option<u32> = None;
	let mut dispatch: Vec<Vec<u8>> = vec![b"refine_ext".to_vec(), b"accumulate_ext".to_vec()];
	let mut keep: Option<String> = None;
	let mut isa = polkavm_linker::TargetInstructionSet::JamV1;
	let mut i = 2;
	while i < args.len() {
		let val = || args.get(i + 1).cloned().unwrap_or_else(|| usage());
		match args[i].as_str() {
			"--name" => name = val(),
			"--version" => version = val(),
			"--license" => license = val(),
			"--author" => authors.push(val()),
			"--min-stack-size" => min_stack = Some(val().parse().expect("stack size")),
			"--dispatch" => dispatch = val().split(',').map(|s| s.as_bytes().to_vec()).collect(),
			"--keep-polkavm" => keep = Some(val()),
			"--isa" =>
				isa = match val().as_str() {
					"jamv1" => polkavm_linker::TargetInstructionSet::JamV1,
					"latest" => polkavm_linker::TargetInstructionSet::Latest,
					_ => usage(),
				},
			_ => usage(),
		}
		i += 2;
	}

	let elf = fs::read(input).unwrap_or_else(|e| {
		eprintln!("cannot read {input}: {e}");
		exit(1)
	});

	let mut config = polkavm_linker::Config::default();
	config.set_strip(true);
	config.set_dispatch_table(dispatch);
	if let Some(s) = min_stack {
		config.set_min_stack_size(s);
	}
	let polkavm_blob = link(config, isa, &elf);
	if let Some(path) = keep {
		fs::write(&path, &polkavm_blob).expect("write .polkavm");
	}

	let parts = polkavm_linker::ProgramParts::from_bytes(polkavm_blob.into()).unwrap_or_else(|e| {
		eprintln!("cannot parse linked program: {e}");
		exit(1)
	});
	let metadata =
		ConventionalMetadata::Info(CrateInfo { name, version, license, authors }).encode();
	let blob = ProgramBlob::from_pvm(&parts, Cow::Owned(metadata)).to_vec().unwrap_or_else(|e| {
		eprintln!("cannot build JAM blob: {e}");
		exit(1)
	});
	fs::write(output, &blob).expect("write output");
	println!(
		"{output}: {} bytes (ro {} / rw {} / stack {} / code+jt {})",
		blob.len(),
		parts.ro_data.len(),
		parts.rw_data.len(),
		parts.stack_size,
		parts.code_and_jump_table.len()
	);
}

fn link(
	config: polkavm_linker::Config,
	isa: polkavm_linker::TargetInstructionSet,
	elf: &[u8],
) -> Vec<u8> {
	polkavm_linker::program_from_elf(config, isa, elf).unwrap_or_else(|e| {
		eprintln!("link failed: {e}");
		exit(1)
	})
}
