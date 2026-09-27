#[cfg(test)]
#[path = "../../../support/portable_cases.rs"]
mod portable;

#[cfg(test)]
#[path = "../../../support/task_cases.rs"]
mod tasks;

#[cfg(test)]
#[path = "../../../support/parallel_cases.rs"]
mod parallel;

#[cfg(test)]
#[path = "../../../support/synchronization_cases.rs"]
mod synchronization;

#[cfg(test)]
#[path = "../../../support/queue_cases.rs"]
mod queue;

#[cfg(test)]
#[path = "../../../support/job_cases.rs"]
mod jobs;

#[test]
fn independent_task_provider_composes_with_tokio_host() -> Result<(), std::io::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(TEST_HOST_WORKERS)
        .build()?;
    runtime.block_on(tasks::task_cases());
    Ok(())
}

// The smallest worker count that exercises a multi-worker host. This is an
// explicit fixture configuration, with no core or application default implied.
#[cfg(test)]
const TEST_HOST_WORKERS: usize = 2;

#[test]
fn same_workflows_on_tokio() -> Result<(), std::io::Error> {
    portable::assert_send_future(portable::workflow().run(std::sync::Arc::new(
        portable::Environment {
            offset: 1,
            ..portable::Environment::default()
        },
    )));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(TEST_HOST_WORKERS)
        .enable_all()
        .build()?;
    runtime.block_on(portable::portable_cases());
    Ok(())
}

#[test]
fn parallel_operations_compose_with_tokio_host() -> Result<(), std::io::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(TEST_HOST_WORKERS)
        .build()?;
    runtime.block_on(parallel::parallel_cases());
    Ok(())
}

#[test]
fn synchronization_composes_with_tokio_host() -> Result<(), std::io::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(TEST_HOST_WORKERS)
        .build()?;
    runtime.block_on(synchronization::synchronization_cases());
    Ok(())
}

#[test]
fn queues_compose_with_tokio_host_and_external_task_provider() -> Result<(), std::io::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(TEST_HOST_WORKERS)
        .build()?;
    runtime.block_on(queue::queue_cases());
    Ok(())
}

#[test]
fn cpu_blocking_and_async_operations_compose_on_tokio_host() -> Result<(), std::io::Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(TEST_HOST_WORKERS).build()?;
    runtime.block_on(jobs::job_cases());
    Ok(())
}
