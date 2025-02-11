use crate::{bb::{FuncBB, BB}, flow::{FlowElem, Flow, And}};
use common::{BinOp, UnOp, HashMap, HashSet, Ref};
use tac::{TacKind, Operand, MemHint, CallKind, Tac, TacIter, TacPayload};
use bitset::traits::*;

pub fn work(f: &mut FuncBB) { WorkCtx::new(f).work(f); }

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
enum TacRhs {
  Bin(BinOp, [Operand; 2]),
  Un(UnOp, [Operand; 1]),
  Load([Operand; 1], i32),
}

impl TacRhs {
  fn from_tac(kind: TacKind) -> Option<TacRhs> {
    match kind {
      TacKind::Bin { op, lr, .. } => Some(TacRhs::Bin(op, lr)),
      TacKind::Un { op, r, .. } => Some(TacRhs::Un(op, r)),
      TacKind::Load { base, off, .. } => { Some(TacRhs::Load(base, off)) }
      _ => None
    }
  }

  fn r(&self) -> &[Operand] {
    match self {
      TacRhs::Bin(_, lr) => lr,
      TacRhs::Un(_, r) => r,
      TacRhs::Load(base, _) => base,
    }
  }
}

// return whether this tac kill (obj, arr)
fn mem_kill(kind: TacKind) -> (bool, bool) {
  match kind {
    TacKind::Store { hint, .. } => match hint {
      MemHint::Immutable => (false, false),
      MemHint::Obj => (true, false),
      MemHint::Arr => (false, true),
    }
    TacKind::Call { kind, .. } => match kind {
      CallKind::Virtual(_, hint) | CallKind::Static(_, hint) => (hint.arg_obj, hint.arg_arr),
      _ => (false, false)
    }
    _ => (false, false)
  }
}

// all Box<[u32]> are bitset of expression(TacRhs) id
struct WorkCtx<'a> {
  // write2id: k -> v: writing to this k can affect the result of TacRhs in v
  write2id: HashMap<u32, Box<[u32]>>,
  rhs2id: HashMap<TacRhs, u32>,
  // tac2id: tac to its TacRhs's id
  tac2id: HashMap<Ref<'a, Tac<'a>>, u32>,
  // obj/arr: these TacRhs are Load, and they load from obj/arr
  obj: Box<[u32]>,
  arr: Box<[u32]>,
  // used in dfs, avoid circular bb link crashing dfs
  vis: Vec<bool>,
}

impl<'a> WorkCtx<'a> {
  pub fn new(f: &FuncBB<'a>) -> WorkCtx<'a> {
    let (mut write2id, mut rhs2id, mut tac2id) = (HashMap::new(), HashMap::new(), HashMap::new());
    let (mut obj, mut arr) = (HashSet::new(), HashSet::new());
    for b in &f.bb {
      for t in b.iter() {
        let payload = t.payload.borrow();
        let payload = &*payload;
        if let Some(rhs) = TacRhs::from_tac(payload.kind) {
          let id = rhs2id.len() as u32;
          let id = *rhs2id.entry(rhs).or_insert(id);
          tac2id.insert(Ref(t), id);
          if let TacKind::Load { hint, .. } = payload.kind {
            match hint {
              MemHint::Immutable => {}
              MemHint::Obj => { obj.insert(id); }
              MemHint::Arr => { arr.insert(id); }
            };
          }
          for r in rhs.r() {
            if let Operand::Reg(r) = r {
              write2id.entry(*r).or_insert_with(HashSet::new).insert(id);
            }
          }
        }
      }
    }
    let (obj, arr) = (iter2bs(&obj, rhs2id.len()), iter2bs(&arr, rhs2id.len()));
    let write2id = write2id.iter().map(|(&k, v)| (k, iter2bs(v, rhs2id.len()))).collect();
    WorkCtx { write2id, rhs2id, tac2id, obj, arr, vis: vec![false; f.bb.len()] }
  }

  pub fn work(&mut self, f: &mut FuncBB<'a>) {
    let mut available_expr_flow = Flow::<And>::new(f.bb.len() + 1, self.rhs2id.len());
    let each = available_expr_flow.each();
    let FlowElem { gen, kill, out, .. } = available_expr_flow.split();
    // add offset 1, leave index 0 as an virtual entry node
    // initial value of out is U, except for entry node
    // entry node has an edge to the first node, that's what `prev_with_entry` does
    for (off, b) in f.bb.iter().enumerate().map(|(idx, bb)| ((idx + 1) * each, bb)) {
      self.compute_gen_kill(b, &mut gen[off..off + each], &mut kill[off..off + each]);
    }
    for x in out.iter_mut().skip(each) { *x = !0; }
    available_expr_flow.solve(f.bb.iter().enumerate().map(|b| b.1.prev_with_entry(b.0 + 1)));
    let FlowElem { in_, .. } = available_expr_flow.split();
    for idx in 0..f.bb.len() { // borrow checker...
      let off = (idx + 1) * each;
      self.do_optimize(idx, f, &mut in_[off..off + each]);
    }
  }

  fn compute_gen_kill(&self, b: &BB, gen: &mut [u32], kill: &mut [u32]) {
    for t in b.iter() {
      let payload = t.payload.borrow();
      let payload = &*payload;
      if let Some(rhs) = TacRhs::from_tac(payload.kind).map(|rhs| self.rhs2id[&rhs]) { gen.bsset(rhs) }
      if let Some(w) = payload.kind.rw().1.and_then(|w| self.write2id.get(&w)) {
        kill.bsor(w);
        gen.bsandn(w); // this has to be done after gen.bsset(rhs), because x = x + y doesn't gen x + y
      }
      let (obj, arr) = mem_kill(payload.kind);
      if obj {
        kill.bsor(&self.obj);
        gen.bsandn(&self.obj);
      }
      if arr {
        kill.bsor(&self.arr);
        gen.bsandn(&self.arr);
      }
    }
  }

  // all available expression with index = `rhs` be replaced by computing it to `new` and copy `new` to original dst
  fn dfs(&mut self, idx: usize, f: &mut FuncBB<'a>, iter: impl IntoIterator<Item=&'a Tac<'a>>, rhs: u32, new: u32) {
    if self.vis[idx] { return; }
    self.vis[idx] = true;
    for t in iter {
      if self.tac2id.get(&Ref(t)) == Some(&rhs) {
        let old = std::mem::replace(t.payload.borrow_mut().kind.rw_mut().1.expect("This tac with rhs must also have a lhs."), new);
        let payload = TacPayload { kind: TacKind::Assign { dst: old, src: [Operand::Reg(new)] } }.into();
        let copy = f.alloc.alloc(Tac { payload, prev: None.into(), next: None.into() });
        f.bb[idx].insert_after(t, copy);
        return;
      }
    }
    for i in 0..f.bb[idx].prev.len() {
      let prev = f.bb[idx].prev[i] as usize;
      self.dfs(prev, f, f.bb[prev].iter().rev(), rhs, new);
    }
  }

  fn do_optimize(&mut self, idx: usize, f: &mut FuncBB<'a>, in_: &mut [u32]) {
    for (t_idx, t) in f.bb[idx].iter().enumerate() {
      let mut payload = t.payload.borrow_mut();
      let payload = &mut *payload;
      let old_kind = payload.kind;
      if let Some(rhs) = TacRhs::from_tac(payload.kind) {
        let rhs = self.rhs2id[&rhs];
        if in_.bsget(rhs) {
          let new = f.new_reg();
          for v in &mut self.vis { *v = false; }
          let prev = TacIter::new(f.bb[idx].first, Some(t), t_idx + 1).rev().skip(1);
          self.dfs(idx, f, prev, rhs, new);
          let w = payload.kind.rw().1.expect("The tac with rhs must also have a lhs.");
          payload.kind = TacKind::Assign { dst: w, src: [Operand::Reg(new)] };
        }
      }
      if let Some(rhs) = TacRhs::from_tac(old_kind).map(|rhs| self.rhs2id[&rhs]) { in_.bsset(rhs) }
      if let Some(w) = old_kind.rw().1.and_then(|w| self.write2id.get(&w)) { in_.bsandn(w) }
      let (obj, arr) = mem_kill(old_kind);
      if obj { in_.bsandn(&self.obj); }
      if arr { in_.bsandn(&self.arr); }
    }
  }
}