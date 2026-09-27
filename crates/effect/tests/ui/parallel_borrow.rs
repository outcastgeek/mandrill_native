use mandrill_effect::{Defect, Effect, HasTaskSpawner, ScheduledTask, TaskSpawner};
use std::num::NonZeroUsize;
struct Environment;
impl HasTaskSpawner for Environment {
    type Tasks = Self;
    fn task_spawner(&self) -> &Self { self }
}
impl TaskSpawner for Environment {
    fn spawn(&self, _: ScheduledTask) -> Result<(), Defect> { Ok(()) }
}
fn main() {
    let _ = Effect::<&String, (), Environment>::for_each_concurrent(NonZeroUsize::MIN, vec![String::new()], |input| {
        Effect::sync(move || input)
    });
}
