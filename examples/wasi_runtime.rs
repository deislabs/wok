extern crate wok;

use std::collections::HashMap;
use std::io::Read;
use wok::wasm::{Runtime, WasiRuntime};

fn main() {
    let mut dirs = HashMap::default();
    dirs.insert(".".into(), None);

    let mut env = HashMap::default();
    env.insert("FOO".into(), "bar".into());

    let args: Vec<String> = vec!["a", "lovely", "bunch", "of", "coconuts"]
        .iter()
        .map(|&s| s.to_owned())
        .collect();

    let runtime = WasiRuntime::new("./examples/printer.wasm", env, args, dirs, "./").unwrap();

    runtime.run().unwrap();
    let (mut stdout_buf, mut stderr_buf) = runtime.output().unwrap();

    let mut stdout = String::default();
    let mut stderr = String::default();

    stdout_buf.read_to_string(&mut stdout).unwrap();
    stderr_buf.read_to_string(&mut stderr).unwrap();

    println!("STDOUT is:\n{}", stdout);
    println!("STDERR is:\n{}", stderr);
}
