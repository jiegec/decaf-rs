mod info;

use syntax::{ast::*, ty::*, ScopeOwner};
use ::tac::{*, self, TacKind, TacKind::*, Operand::*, Intrinsic::*};
use common::{Ref, MAIN_METHOD, BinOp::*, UnOp::*, IndexSet, IndexMap, HashMap};
use typed_arena::Arena;
use crate::info::*;

#[derive(Default)]
struct TacGen<'a> {
  reg_cnt: u32,
  label_cnt: u32,
  loop_stk: Vec<u32>,
  // Id & Index will behave differently when they are the lhs of an assignment
  // cur_assign contains the current assign rhs operand, or None if the current handling expr doesn't involve in assign
  cur_assign: Option<Operand>,
  str_pool: IndexSet<&'a str>,
  var_info: HashMap<Ref<'a, VarDef<'a>>, VarInfo>,
  func_info: HashMap<Ref<'a, FuncDef<'a>>, FuncInfo>,
  class_info: HashMap<Ref<'a, ClassDef<'a>>, ClassInfo<'a>>,
}

pub fn work<'a>(p: &'a Program<'a>, alloc: &'a Arena<Tac<'a>>) -> TacProgram<'a> {
  TacGen::default().program(p, alloc)
}

impl<'a> TacGen<'a> {
  fn program(mut self, p: &Program<'a>, alloc: &'a Arena<Tac<'a>>) -> TacProgram<'a> {
    let mut tp = TacProgram::default();
    for (idx, &c) in p.class.iter().enumerate() {
      self.define_str(c.name);
      self.resolve_field(c);
      self.class_info.get_mut(&Ref(c)).unwrap().idx = idx as u32;
      tp.func.push(self.build_new(c, alloc));
    }
    {
      let mut idx = tp.func.len() as u32; // their are already some `_Xxx_New` functions in tp.func
      for &c in &p.class {
        for &f in &c.field {
          if let FieldDef::FuncDef(f) = f {
            self.func_info.get_mut(&Ref(f)).unwrap().idx = idx;
            idx += 1;
          }
        }
      }
    }
    for &c in &p.class {
      for f in &c.field {
        if let FieldDef::FuncDef(fu) = f {
          let this = if fu.static_ { 0 } else { 1 };
          for (idx, p) in fu.param.iter().enumerate() {
            self.var_info.insert(Ref(p), VarInfo { off: idx as u32 + this });
          }
          // these regs are occupied by parameters
          self.reg_cnt = fu.param.len() as u32 + this;
          self.label_cnt = 0;
          let mut f = TacFunc::empty(alloc, if Ref(c) == Ref(p.main.get().unwrap()) && fu.name == MAIN_METHOD {
            FuncNameKind::Main
          } else {
            FuncNameKind::Member { class: c.name, func: fu.name }
          }, self.reg_cnt, fu.ret_ty() != Ty::void());
          self.block(&fu.body, &mut f);
          f.max_reg = self.reg_cnt;
          // add an return at the end of return-void function
          if fu.ret_ty() == Ty::void() {
            f.push(TacKind::Ret { src: None });
          }
          tp.func.push(f);
        }
      }
    }
    for &c in &p.class {
      tp.vtbl.push(tac::VTbl {
        parent: c.parent_ref.get().map(|p| self.class_info[&Ref(p)].idx),
        class: c.name,
        func: self.class_info[&Ref(c)].vtbl.iter().map(|(_, &f)| self.func_info[&Ref(f)].idx).collect(),
      });
    }
    tp.str_pool = self.str_pool;
    tp
  }

  fn block(&mut self, b: &Block<'a>, f: &mut TacFunc<'a>) {
    for s in &b.stmt { self.stmt(s, f); }
  }

  fn stmt(&mut self, s: &Stmt<'a>, f: &mut TacFunc<'a>) {
    use StmtKind::*;
    match &s.kind {
      Assign(a) => {
        self.cur_assign = Some(self.expr(&a.src, f));
        self.expr(&a.dst, f);
      }
      LocalVarDef(v) => {
        let reg = self.reg();
        self.var_info.insert(Ref(v), VarInfo { off: reg });
        let init = v.init.as_ref().map(|(_, e)| self.expr(e, f)).unwrap_or(Const(0));
        f.push(TacKind::Assign { dst: reg, src: [init] });
      }
      ExprEval(e) => { self.expr(e, f); }
      Skip(_) => {}
      If(i) => {
        let before_else = self.label();
        let cond = self.expr(&i.cond, f);
        f.push(Jif { label: before_else, z: true, cond: [cond] });
        self.block(&i.on_true, f);
        if let Some(of) = &i.on_false {
          let after_else = self.label();
          f.push(Jmp { label: after_else });
          f.push(Label { label: before_else });
          self.block(of, f);
          f.push(Label { label: after_else });
        } else {
          f.push(Label { label: before_else });
        }
      }
      While(w) => {
        //   jump before_cond
        // before_body:
        //   body
        // before_cond:
        //   compute cond
        //   if cond jump before_body
        // after_body: (for break's use)
        let (before_cond, before_body, after_body) = (self.label(), self.label(), self.label());
        self.loop_stk.push(after_body);
        f.push(Jmp { label: before_cond });
        f.push(Label { label: before_body });
        self.block(&w.body, f);
        f.push(Label { label: before_cond });
        let cond = self.expr(&w.cond, f);
        f.push(Jif { label: before_body, z: false, cond: [cond] });
        f.push(Label { label: after_body });
        self.loop_stk.pop();
      }
      For(fo) => {
        // init
        //   jump before_cond
        // before_body:
        //   body
        //   update
        // before_cond:
        //   compute cond
        //   if cond jump before_body
        // after_body: (for break's use)
        let (before_cond, before_body, after_body) = (self.label(), self.label(), self.label());
        self.loop_stk.push(after_body);
        self.stmt(&fo.init, f);
        f.push(Jmp { label: before_cond });
        f.push(Label { label: before_body });
        self.block(&fo.body, f);
        self.stmt(&fo.update, f);
        f.push(Label { label: before_cond });
        let cond = self.expr(&fo.cond, f);
        f.push(Jif { label: before_body, z: false, cond: [cond] });
        f.push(Label { label: after_body });
        self.loop_stk.pop();
      }
      Return(r) => if let Some(e) = r {
        let ret = self.expr(e, f);
        f.push(Ret { src: Some([ret]) });
      } else {
        f.push(Ret { src: None });
      }
      Print(p) => for e in p {
        let reg = self.expr(e, f);
        f.push(Param { src: [reg] });
        match e.ty.get() {
          t if t == Ty::int() => { self.intrinsic(_PrintInt, f); }
          t if t == Ty::bool() => { self.intrinsic(_PrintBool, f); }
          t if t == Ty::string() => { self.intrinsic(_PrintString, f); }
          t => unreachable!("Shouldn't meet type {:?} in Print in these phase, type checking should have reported error.", t),
        }
      }
      Break(_) => { f.push(Jmp { label: *self.loop_stk.last().unwrap() }); }
      Block(b) => self.block(b, f),
    }
  }

  fn expr(&mut self, e: &Expr<'a>, f: &mut TacFunc<'a>) -> Operand {
    use ExprKind::*;
    let assign = self.cur_assign.take();
    match &e.kind {
      VarSel(v) => Reg({
        let var = v.var.get().unwrap();
        let off = self.var_info[&Ref(var)].off;
        match var.owner.get().unwrap() {
          ScopeOwner::Local(_) | ScopeOwner::Param(_) => if let Some(src) = assign { // off is register
            // don't care this return, the below 0 is the same
            (f.push(TacKind::Assign { dst: off, src: [src] }), 0).1
          } else { off }
          ScopeOwner::Class(_) => { // off is offset
            // `this` is at argument 0
            let owner = v.owner.as_ref().map(|o| self.expr(o, f)).unwrap_or(Reg(0));
            match assign {
              Some(src) => (f.push(Store { src_base: [src, owner], off: off as i32 * INT_SIZE, hint: MemHint::Obj }), 0).1,
              None => {
                let dst = self.reg();
                (f.push(Load { dst, base: [owner], off: off as i32 * INT_SIZE, hint: MemHint::Obj }), dst).1
              }
            }
          }
          ScopeOwner::Global(_) => unreachable!("Impossible to declare a variable in global scope."),
        }
      }),
      IndexSel(i) => Reg({
        let (arr, idx) = (self.expr(&i.arr, f), self.expr(&i.idx, f));
        let (ok, len, cmp) = (self.reg(), self.length(arr, f), self.reg());
        let (err, after) = (self.label(), self.label());
        f.push(Bin { op: Ge, dst: ok, lr: [idx, Const(0)] })
          .push(Bin { op: Lt, dst: cmp, lr: [idx, Reg(len)] })
          .push(Bin { op: And, dst: ok, lr: [Reg(ok), Reg(cmp)] })
          .push(Jif { label: err, z: true, cond: [Reg(ok)] });
        // range check passed if reach here
        let off = self.reg();
        f.push(Bin { op: Mul, dst: off, lr: [idx, Const(INT_SIZE)] })
          .push(Bin { op: Add, dst: off, lr: [Reg(off), arr] });
        let ret = match assign {
          Some(src) => (f.push(Store { src_base: [src, Reg(off)], off: 0, hint: MemHint::Arr }), 0).1,
          None => {
            let reg = self.reg();
            (f.push(Load { dst: reg, base: [Reg(off)], off: 0, hint: MemHint::Arr }), reg).1
          }
        };
        f.push(Jmp { label: after });
        self.re(INDEX_OUT_OF_BOUND, f.push(Label { label: err }));
        f.push(Label { label: after });
        ret
      }),
      IntLit(i) => Const(*i),
      BoolLit(b) => Const(*b as i32),
      StringLit(s) => {
        let s1 = self.reg();
        f.push(LoadStr { dst: s1, s: self.define_str(s) });
        Reg(s1)
      }
      NullLit(_) => Const(0),
      Call(c) => {
        let v = if let ExprKind::VarSel(v) = &c.func.kind { v } else { unimplemented!() };
        Reg(match &v.owner {
          Some(o)if o.ty.get().is_arr() => {
            let arr = self.expr(o, f);
            self.length(arr, f)
          }
          _ => {
            let fu = c.func_ref.get().unwrap();
            let ret = if fu.ret_ty() != Ty::void() { Some(self.reg()) } else { None };
            let args = c.arg.iter().map(|a| self.expr(a, f)).collect::<Vec<_>>();
            let hint = CallHint {
              arg_obj: c.arg.iter().any(|a| a.ty.get().is_class()) || !fu.static_,
              arg_arr: c.arg.iter().any(|a| a.ty.get().arr > 0),
            };
            if fu.static_ {
              for a in args {
                f.push(Param { src: [a] });
              }
              f.push(TacKind::Call { dst: ret, kind: CallKind::Static(self.func_info[&Ref(fu)].idx, hint) });
            } else {
              let owner = match &v.owner {
                Some(o) => self.expr(o, f),
                None => Reg(0), // this(i.owner is not set during typeck)
              };
              f.push(Param { src: [owner] });
              for a in args {
                f.push(Param { src: [a] });
              }
              let slot = self.reg();
              let off = self.func_info[&Ref(fu)].off;
              f.push(Load { dst: slot, base: [owner], off: 0, hint: MemHint::Immutable })
                .push(Load { dst: slot, base: [Reg(slot)], off: off as i32 * INT_SIZE, hint: MemHint::Immutable });
              f.push(TacKind::Call { dst: ret, kind: CallKind::Virtual([Reg(slot)], hint) });
            }
            ret.unwrap_or(0) // if ret is None, the result can't be assigned to others, so 0 will not be used
          }
        })
      }
      Unary(u) => {
        let (r, dst) = (self.expr(&u.r, f), self.reg());
        (f.push(Un { op: u.op, dst, r: [r] }), Reg(dst)).1
      }
      Binary(b) => {
        let (l, r) = (self.expr(&b.l, f), self.expr(&b.r, f));
        match b.op {
          Eq | Ne if b.l.ty.get() == Ty::string() => {
            f.push(Param { src: [l] }).push(Param { src: [r] });
            let dst = self.intrinsic(_StringEqual, f).unwrap();
            if b.op == Ne {
              f.push(Un { op: Not, dst, r: [Reg(dst)] });
            }
            Reg(dst)
          }
          op => {
            let dst = self.reg();
            (f.push(Bin { op, dst, lr: [l, r] }), Reg(dst)).1
          }
        }
      }
      This(_) => Reg(0),
      ReadInt(_) => Reg(self.intrinsic(_ReadInt, f).unwrap()),
      ReadLine(_) => Reg(self.intrinsic(_ReadLine, f).unwrap()),
      NewClass(n) => Reg({
        let dst = self.reg();
        // by design, a class's new func in functions have the same index as its vtbl in vtbls
        f.push(TacKind::Call { dst: Some(dst), kind: CallKind::Static(self.class_info[&Ref(n.class.get().unwrap())].idx, CallHint { arg_obj: false, arg_arr: false }) });
        dst
      }),
      NewArray(n) => Reg({
        let len = self.expr(&n.len, f);
        let (ok, before_cond, before_body) = (self.label(), self.label(), self.label());
        let (cmp, ptr) = (self.reg(), self.reg());
        f.push(Bin { op: Lt, dst: cmp, lr: [len, Const(0)] })
          .push(Jif { label: ok, z: true, cond: [Reg(cmp)] });
        self.re(NEW_ARR_NEG, f);
        f.push(Label { label: ok });
        let arr = self.intrinsic(_Alloc, f
          .push(Bin { op: Mul, dst: ptr, lr: [len, Const(INT_SIZE)] })
          .push(Bin { op: Add, dst: ptr, lr: [Reg(ptr), Const(INT_SIZE)] }) // now ptr = bytes to allocate
          .push(Param { src: [Reg(ptr)] })).unwrap();
        f.push(Bin { op: Add, dst: ptr, lr: [Reg(arr), Reg(ptr)] }); // now ptr = end of array
        f.push(Bin { op: Add, dst: arr, lr: [Reg(arr), Const(INT_SIZE)] }); // now arr = begin of array([0])
        f.push(Jmp { label: before_cond }) // loop(reversely), set all to 0
          .push(Label { label: before_body })
          .push(Bin { op: Sub, dst: ptr, lr: [Reg(ptr), Const(INT_SIZE)] })
          .push(Store { src_base: [Const(0), Reg(ptr)], off: 0, hint: MemHint::Arr })
          .push(Label { label: before_cond })
          .push(Bin { op: Eq, dst: cmp, lr: [Reg(ptr), Reg(arr)] })
          .push(Jif { label: before_body, z: true, cond: [Reg(cmp)] }); // when ptr == arr, loop end
        f.push(Store { src_base: [len, Reg(arr)], off: -INT_SIZE, hint: MemHint::Immutable }); // arr[-1] = len
        arr
      }),
      ClassTest(t) => Reg({
        let obj = self.expr(&t.expr, f);
        self.check_cast(obj, self.class_info[&Ref(t.class.get().unwrap())].idx, f)
      }),
      ClassCast(t) => {
        let obj = self.expr(&t.expr, f);
        let check = self.check_cast(obj, self.class_info[&Ref(t.class.get().unwrap())].idx, f);
        let (msg, vtbl, ok) = (self.reg(), self.reg(), self.label());
        f.push(Jif { label: ok, z: false, cond: [Reg(check)] });
        let s = self.define_str(BAD_CAST1); // borrow checker...
        self.intrinsic(_PrintString, f.push(LoadStr { dst: msg, s }).push(Param { src: [Reg(msg)] }));
        self.intrinsic(_PrintString, f.push(Load { dst: vtbl, base: [obj], off: 0, hint: MemHint::Immutable })
          .push(Load { dst: msg, base: [Reg(vtbl)], off: INT_SIZE as i32, hint: MemHint::Immutable }).push(Param { src: [Reg(msg)] }));
        let s = self.define_str(BAD_CAST2);
        self.intrinsic(_PrintString, f.push(LoadStr { dst: msg, s }).push(Param { src: [Reg(msg)] }));
        let s = self.define_str(t.name);
        self.intrinsic(_PrintString, f.push(LoadStr { dst: msg, s }).push(Param { src: [Reg(msg)] }));
        let s = self.define_str(BAD_CAST3);
        self.intrinsic(_PrintString, f.push(LoadStr { dst: msg, s }).push(Param { src: [Reg(msg)] }));
        self.intrinsic(_Halt, f);
        f.push(Label { label: ok });
        obj
      }
    }
  }
}

impl<'a> TacGen<'a> {
  fn define_str(&mut self, s: &'a str) -> u32 { self.str_pool.insert_full(s).0 as u32 }

  fn reg(&mut self) -> u32 { (self.reg_cnt, self.reg_cnt += 1).0 }

  fn label(&mut self) -> u32 { (self.label_cnt, self.label_cnt += 1).0 }

  // if you don't need to modify the returned register, it is more recommended to use Const(i)
  fn int(&mut self, i: i32, f: &mut TacFunc<'a>) -> u32 {
    let ret = self.reg();
    f.push(LoadInt { dst: ret, i });
    ret
  }

  fn intrinsic(&mut self, i: Intrinsic, f: &mut TacFunc<'a>) -> Option<u32> {
    let ret = if i.has_ret() { Some(self.reg()) } else { None };
    f.push(TacKind::Call { dst: ret, kind: CallKind::Intrinsic(i) });
    ret
  }

  fn length(&mut self, arr: Operand, f: &mut TacFunc<'a>) -> u32 {
    let ret = self.reg();
    f.push(Load { dst: ret, base: [arr], off: -(INT_SIZE as i32), hint: MemHint::Immutable });
    ret
  }

  // re for runtime error
  fn re(&mut self, msg: &'static str, f: &mut TacFunc<'a>) {
    let src = self.reg();
    let s = self.define_str(msg);
    self.intrinsic(_PrintString, f.push(LoadStr { dst: src, s }).push(Param { src: [Reg(src)] }));
    self.intrinsic(_Halt, f);
  }

  fn check_cast(&mut self, obj: Operand, vtbl_idx: u32, f: &mut TacFunc<'a>) -> u32 {
    // ret = 0
    // while (cur)
    //   ret = (cur == target)
    //   if ret = 1
    //     break
    //   cur = cur->parent
    let (ret, cur, target) = (self.int(0, f), self.reg(), self.reg());
    let (before_cond, after_body) = (self.label(), self.label());
    f.push(LoadVTbl { dst: target, v: vtbl_idx });
    f.push(Load { dst: cur, base: [obj], off: 0, hint: MemHint::Immutable });
    f.push(Label { label: before_cond });
    f.push(Jif { label: after_body, z: true, cond: [Reg(cur)] });
    f.push(Bin { op: Eq, dst: ret, lr: [Reg(cur), Reg(target)] }).push(Jif { label: after_body, z: false, cond: [Reg(ret)] });
    f.push(Load { dst: cur, base: [Reg(cur)], off: 0, hint: MemHint::Immutable });
    f.push(Jmp { label: before_cond });
    f.push(Label { label: after_body });
    ret
  }
}

impl<'a> TacGen<'a> {
  // `idx` in ClassInfo & FuncInfo is not determined here, just use 0
  // it is guaranteed that all functions (static/virtual) are inserted into self.func_info
  // this function depends on the fact that no cyclic inheritance exist, which is guaranteed in typeck
  fn resolve_field(&mut self, c: &'a ClassDef<'a>) {
    if !self.class_info.contains_key(&Ref(c)) {
      let (mut field_cnt, mut vtbl) = if let Some(p) = c.parent_ref.get() {
        self.resolve_field(p);
        let p = &self.class_info[&Ref(p)];
        (p.field_cnt, p.vtbl.clone())
      } else {
        // starting from 1, partly because of vtbl is at 0, mainly because > 0 can be used to determine whether already resolved
        (1, IndexMap::default())
      };
      for f in &c.field {
        match f {
          FieldDef::FuncDef(f) => if !f.static_ {
            if let Some((idx, _, p_f)) = vtbl.get_full_mut(f.name) {
              // + 2 for 0 is parent vtbl, 1 is class name
              self.func_info.insert(Ref(f), FuncInfo { off: idx as u32 + 2, idx: 0 });
              *p_f = f; // override
            } else {
              self.func_info.insert(Ref(f), FuncInfo { off: vtbl.len() as u32 + 2, idx: 0 });
              vtbl.insert(f.name, f);
            }
          } else {
            // off is useless for static functions
            self.func_info.insert(Ref(f), FuncInfo { off: 0, idx: 0 });
          }
          FieldDef::VarDef(v) => {
            self.var_info.insert(Ref(v), VarInfo { off: field_cnt });
            field_cnt += 1;
          }
        }
      }
      self.class_info.insert(Ref(c), ClassInfo { field_cnt, idx: 0, vtbl });
    }
  }

  fn build_new(&mut self, c: &'a ClassDef<'a>, alloc: &'a Arena<Tac<'a>>) -> TacFunc<'a> {
    self.reg_cnt = 0;
    let ClassInfo { field_cnt, idx, .. } = self.class_info[&Ref(c)];
    let mut f = TacFunc::empty(alloc, FuncNameKind::New { class: c.name }, 0, true);
    f.push(Param { src: [Const(field_cnt as i32 * INT_SIZE)] });
    let ret = self.intrinsic(_Alloc, &mut f).unwrap();
    let vtbl = self.reg();
    f.push(LoadVTbl { dst: vtbl, v: idx });
    f.push(Store { src_base: [Reg(vtbl), Reg(ret)], off: 0, hint: MemHint::Immutable });
    for i in 1..field_cnt {
      f.push(Store { src_base: [Const(0), Reg(ret)], off: i as i32 * INT_SIZE, hint: MemHint::Obj });
    }
    f.push(Ret { src: Some([Reg(ret)]) });
    f.max_reg = self.reg_cnt;
    f
  }
}
