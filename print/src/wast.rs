use common::{IndentPrinter, IgnoreResult};
use tac::{TacProgram, FuncNameKind};
use codegen::mips::AsmTemplate;
use std::fmt::Write;

fn to_wasm_int(num: usize) -> String {
  format!("\"\\{:02X}\\{:02X}\\{:02X}\\{:02X}\"", (num) & 0xFF, (num >> 8) & 0xFF, (num >> 16) & 0xFF, (num >> 24) & 0xFF)
}

fn to_wasm_string(s: &str) -> String {
  let mut result = String::new();
  result.push_str("\"");
  for byte in s.as_bytes() {
    if *byte >= 0x20 && *byte <= 0x7e {
      result.push(*byte as char);
    } else {
      result.push_str(&format!("\\{:02X}", byte));
    }
  }
  result.push_str("\"");
  result
}

pub fn data(pr: &TacProgram, p: &mut IndentPrinter) {
  write!(p, "(module").ignore();
  p.inc();
  write!(p, "(memory 1)").ignore();
  write!(p, "(export \"memory\" (memory 0))").ignore();
  let mut offsets = Vec::new();
  let mut offset = 0;
  for v in &pr.vtbl {
    let (_, name) = pr.str_pool.get_full(v.class).expect("tacgen should have put class name into `str_pool`");
    let size = 4 + 4 + v.func.len() * 4 + name.len();
    offsets.push(offset);
    offset += size;
  }

  offset = 0;
  for v in &pr.vtbl {
    if let Some(pa) = v.parent {
      write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(offsets[pa as usize])).ignore();
    } else {
      write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(0)).ignore();
    }
    offset += 4;
    write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(offset + 4 + v.func.len() * 4)).ignore();
    offset += 4;
    for &f in &v.func {
      write!(p, "(data (i32.const {}) {})", offset, to_wasm_string(&format!("{:?}", pr.func[f as usize].name))).ignore();
      offset += 4;
    }
    let (_, name) = pr.str_pool.get_full(v.class).expect("tacgen should have put class name into `str_pool`");
    write!(p, "(data (i32.const {}) {})", offset, to_wasm_string(name)).ignore();
    offset += name.len();
  }
  writeln!(p).ignore();
  for (idx, s) in pr.str_pool.iter().enumerate() {
    write!(p, "(func $_STRING{} (result i32)", idx).ignore();
    p.indent(|p| write!(p, "(i32.const {}))", offset).ignore());
    write!(p, "(data (i32.const {}) {})", offset, to_wasm_string(s)).ignore();
    offset += s.len();
  }
  writeln!(p).ignore();
}

pub fn func(f: &[AsmTemplate], name: FuncNameKind, p: &mut IndentPrinter) {
    /*
  write!(p, ".text").ignore();
  write!(p, ".globl {:?}", name).ignore();
  write!(p, "{:?}:", name).ignore();
  p.indent(|p| for asm in f { write!(p, "{:?}", asm).ignore(); });
  writeln!(p).ignore();
  */
}
