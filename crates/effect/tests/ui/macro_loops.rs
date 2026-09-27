use mandrill_effect::{Effect,effect};
struct Owned(String);
fn main() {
 let _:Effect<String,(),()>=effect! {
  let mut value=Owned(String::new());
  'outer: for index in 0..3 {
   let next=Effect::sync(move||Some(index))?;
   let Some(index)=next else { return value.0; };
   if index==0 { continue; }
   loop { value.0.push('a'); break 'outer; }
  }
  value.0
 };
}
