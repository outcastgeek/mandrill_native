use mandrill_effect::{Defect, Effect, HasTaskSpawner, LibraryDefect, ScheduledTask, Scope, TaskSpawner};
use std::cell::Cell;

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
// No Clone or Sync payload/error requirement, including shared observation.
struct Payload(Cell<()>);
fn main() {
    let _: Effect<bool, Payload, Environment> = Scope::scoped(|scope| {
        scope.fork(Effect::sync(|| Payload(Cell::new(()))))
            .flat_map(|fiber| fiber.await_exit_shared().map(|view| view.with(|outcome| {
                matches!(outcome, mandrill_effect::Exit::Success(_))
            }).is_ok()))
    });
}
