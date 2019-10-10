use common::{BinOp, IgnoreResult, UnOp};
use std::fmt;
use tac::{Operand, CallKind, Intrinsic};

pub type Reg = u32;
type Imm = i32;

pub enum AsmTemplate {
  Bin(BinOp, Reg, Operand, Operand),
  Un(UnOp, Reg, Operand),
  Mv(Reg, Operand),
  Param(Operand),
  CallStatic(Option<u32>, String),
  CallVirtual(Option<u32>, Operand),
  CallIntrinsic(Option<u32>, Intrinsic),
  Ret(Option<Operand>),
  Lw(Reg /* dst */, Operand /* base */, Imm),
  Sw(Operand /* src */, Operand /* base */, Imm),
  Li(Reg, Imm),
  La(Reg, String),
  Label(String),
}

impl fmt::Debug for AsmTemplate {
  fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
    use AsmTemplate::*;
    match self {
      Bin(op, w1, r1, r2) => write!(f, "(set_local {} ({} {} {}))", w1, bin_str(op), operand_str(r1), operand_str(r2)),
      Un(op, w1, r1) => write!(f, "(set_local {} ({} {}))", w1, un_str(op), operand_str(r1)),
      Mv(w1, r1) => write!(f, "(set_local {} {})", w1, operand_str(r1)),
      Param(r1) => write!(f, "{}", operand_str(r1)),
      CallStatic(dst, fun) => {
        if let Some(dst) = dst {
          write!(f, "(set_local {} (call ${}))", dst, fun)
        } else {
          write!(f, "(drop (call ${}))", fun)
        }
      },
      Lw(dst, base, imm) => write!(f, "(set_local {} (i32.load (i32.add {} (i32.const {}))))", dst, operand_str(base), imm),
      Sw(src, base, imm) => write!(f, "(i32.store (i32.add {} (i32.const {})) {})", operand_str(base), imm, operand_str(src)),
      La(dst, addr) => write!(f, "(set_local {} (call ${}))", dst, addr),
      Ret(ret) => match ret {
        Some(op) => {
          write!(f, "{}", operand_str(op))
        }
        None => write!(f, "(i32.const 0)")
      },
      _ => Ok(())
    }
  }
}

pub fn operand_str(operand: &Operand) -> String {
  match operand {
    Operand::Const(i) => format!("(i32.const {})", i),
    Operand::Reg(i) => format!("(get_local {})", i),
  }
}

pub fn bin_str(op: &BinOp) -> &'static str {
  match op {
    BinOp::Add => "i32.add",
    BinOp::Sub => "i32.sub",
    BinOp::Mul => "i32.mul",
    BinOp::Div => "i32.div",
    BinOp::Mod => "i32.mod",
    BinOp::And => "i32.and",
    BinOp::Or => "i32.or",
    BinOp::Eq => "i32.eq",
    BinOp::Ne => "i32.ne",
    BinOp::Lt => "i32.lt",
    BinOp::Le => "i32.le",
    BinOp::Gt => "i32.gt",
    BinOp::Ge => "i32.ge",
  }
}

pub fn un_str(op: &UnOp) -> &'static str {
  match op {
    UnOp::Neg => "i32.neg",
    UnOp::Not => "i32.not",
  }
}