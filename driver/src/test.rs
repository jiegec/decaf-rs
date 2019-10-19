use driver::*;

fn main() {
  let mut fail = false;
  for result in test_all("testcase/S1", Pa::Pa1a).unwrap() {
    println!("{:?}", result);
    if let ResultKind::Pass = result.kind {
    } else {
      fail = true;
    }
  }
  for result in test_all("testcase/S5", Pa::Pa5).unwrap() {
    println!("{:?}", result);
    if let ResultKind::Pass = result.kind {
    } else {
      fail = true;
    }
  }
  for result in test_all("testcase/S5", Pa::Pa5Wast).unwrap() {
    println!("{:?}", result);
    if let ResultKind::Pass = result.kind {
    } else {
      fail = true;
    }
  }
  std::process::exit(if fail { 1 } else { 0 })
}
