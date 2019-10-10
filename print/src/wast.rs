use common::{IndentPrinter, IgnoreResult};
use tac::{TacProgram, FuncNameKind, TacFunc};
use codegen::wast::AsmTemplate;
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
  result.push_str("\\00\"");
  result
}

fn wasm_string_len(s: &str) -> usize {
  ((s.len() + 1) + 3) & !3
}

pub fn data(pr: &TacProgram, p: &mut IndentPrinter) {
  write!(p, "(module").ignore();
  p.inc();
  write!(p, "(import \"wasi_unstable\" \"fd_write\" (func $fd_write (param i32 i32 i32 i32) (result i32)))").ignore();
  write!(p, "(memory 1)").ignore();
  write!(p, "(export \"memory\" (memory 0))").ignore();

  let mut offsets = Vec::new();
  let mut offset = 0;
  for v in &pr.vtbl {
    let (_, name) = pr.str_pool.get_full(v.class).expect("tacgen should have put class name into `str_pool`");
    let size = 4 + 4 + v.func.len() * 4 + wasm_string_len(name);
    offsets.push(offset);
    offset += size;
  }
  // Total static data extent
  let extent = offset + 4;
  offset = 0;
  write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(extent)).ignore();
  offset += 4;

  for v in &pr.vtbl {
    // VTbl symbol as a func
    write!(p, "(func $_{} (result i32)", v.class).ignore();
    p.indent(|p| write!(p, "(i32.const {}))", offset).ignore());

    // parent
    if let Some(pa) = v.parent {
      write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(offsets[pa as usize])).ignore();
    } else {
      write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(0)).ignore();
    }
    offset += 4;
    // name ptr
    write!(p, "(data (i32.const {}) {})", offset, to_wasm_int(offset + 4 + v.func.len() * 4)).ignore();
    offset += 4;
    // funcs
    for &f in &v.func {
      write!(p, "(data (i32.const {}) {})", offset, to_wasm_string(&format!("{:?}", pr.func[f as usize].name))).ignore();
      offset += 4;
    }
    let (_, name) = pr.str_pool.get_full(v.class).expect("tacgen should have put class name into `str_pool`");
    // name
    write!(p, "(data (i32.const {}) {})", offset, to_wasm_string(name)).ignore();
    offset += wasm_string_len(name);
  }
  writeln!(p).ignore();
  for (idx, s) in pr.str_pool.iter().enumerate() {
    write!(p, "(func $_STRING{} (result i32)", idx).ignore();
    p.indent(|p| write!(p, "(i32.const {}))", offset).ignore());
    write!(p, "(data (i32.const {}) {})", offset, to_wasm_string(s)).ignore();
    offset += s.len() + 1;
  }
  writeln!(p).ignore();
}

pub fn func(f: &[AsmTemplate], name: FuncNameKind, p: &mut IndentPrinter, fun: &TacFunc) {
  let mut func_declaration = format!("(func ${:?} (", name);
  if fun.param_num > 0 {
    func_declaration.push_str("param");
    for _ in 0..fun.param_num {
      func_declaration.push_str(" i32)");
    }
    func_declaration.push_str(" (");
  }
  func_declaration.push_str("result i32)");
  write!(p, "{}", func_declaration).ignore();
  p.indent(|p| {
    if fun.max_reg > 0 {
      let mut locals = String::new();
      locals.push_str("(local");
      // TODO
      for _ in 0..32 {
        locals.push_str(" i32");
      }
      locals.push_str(")");
      write!(p, "{}", locals).ignore();
    }

    for asm in f {
      write!(p, "{:?}", asm).ignore();
    }
    write!(p, ")").ignore();
  });
  writeln!(p).ignore();
}
