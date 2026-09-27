use mandrill_effect::{Defect, Effect, HasTaskSpawner, LibraryDefect, ScheduledTask, TaskSpawner};
use std::cell::Cell;
use std::num::NonZeroUsize;
use std::sync::Arc;
struct Provider;
impl TaskSpawner for Provider {
    fn spawn(&self, task: ScheduledTask) -> Result<(), Defect> {
        drop(task);
        Err(Defect::library(LibraryDefect::TaskSubmissionRejected))
    }
}
struct Environment(Provider);
impl HasTaskSpawner for Environment {
    type Tasks = Provider;
    fn task_spawner(&self) -> &Provider { &self.0 }
}
struct Input(String); // No Clone.
struct Output(Cell<usize>); // No Clone or Sync.
struct Error(Cell<()>); // No Clone or Sync.
fn main() {
    let inputs: Arc<[Input]> = vec![Input(String::new())].into();
    let left: Effect<Vec<Output>, Error, Environment> = Effect::for_each_concurrent_shared(NonZeroUsize::MIN, inputs, |input| {
        let length = input.0.len();
        Effect::sync(move || Output(Cell::new(length)))
    });
    let right: Effect<Vec<Output>, Error, Environment> = Effect::all_concurrent(NonZeroUsize::MIN, vec![Effect::sync(|| Output(Cell::new(0)))]);
    let _ = left.race(right);
}
