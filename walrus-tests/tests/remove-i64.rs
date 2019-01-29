use std::path::Path;
use walrus_tests_utils::{wasm2wat, wat2wasm};

fn run(wat_path: &Path) -> Result<(), failure::Error> {
    let wasm = wat2wasm(wat_path);
    let mut module = walrus::module::ModuleConfig::new()
        .generate_names(true)
        .parse(&wasm)?;
    walrus::passes::remove_i64::run(&mut module)?;
    let out_wasm_file = wat_path.with_extension("out.wasm");
    module.emit_wasm_file(&out_wasm_file)?;

    let out_wat = wasm2wat(&out_wasm_file);
    let checker = walrus_tests::FileCheck::from_file(wat_path);
    checker.check(&out_wat);
    Ok(())
}

include!(concat!(env!("OUT_DIR"), "/remove-i64.rs"));
