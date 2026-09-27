use crate::internal::workflow::{self, bind, success};
use crate::{Effect, Release, ResourceValue, Scope};
use std::sync::Arc;

pub struct Layer<I: Send + Sync + 'static, E: Send + 'static, O: Send + 'static> {
    build: Arc<dyn Fn(Scope<E>) -> Effect<O, E, I> + Send + Sync>,
}
impl<I: Send + Sync + 'static, E: Send + 'static, O: Send + 'static> Clone for Layer<I, E, O> {
    fn clone(&self) -> Self {
        Self {
            build: Arc::clone(&self.build),
        }
    }
}
impl<I: Send + Sync + 'static, E: Send + 'static, O: Send + 'static> Layer<I, E, O> {
    #[must_use]
    pub fn from_effect(program: Effect<O, E, I>) -> Self {
        Self::new(move |_| program.clone())
    }
    #[must_use]
    pub fn make_with(factory: impl Fn() -> O + Send + Sync + 'static) -> Self {
        Self::from_effect(Effect::sync(factory))
    }
    #[must_use]
    pub fn new(build: impl Fn(Scope<E>) -> Effect<O, E, I> + Send + Sync + 'static) -> Self {
        Self {
            build: Arc::new(build),
        }
    }
    pub fn build(&self, scope: Scope<E>) -> Effect<O, E, I> {
        let build = Arc::clone(&self.build);
        Effect::defer(move || build(scope.clone()))
    }
    #[must_use]
    pub fn merge<P: Send + 'static, Q: Send + 'static>(
        self,
        right: Layer<I, E, P>,
        combine: impl Fn(O, P) -> Q + Send + Sync + 'static,
    ) -> Layer<I, E, Q> {
        let combine = Arc::new(combine);
        Layer::new(move |scope| {
            let left = self.build(scope.clone());
            let right = right.build(scope);
            let combine = Arc::clone(&combine);
            workflow::reusable(move || {
                let right = right.clone();
                let combine = Arc::clone(&combine);
                bind(left.clone(), move |a| {
                    bind(right, move |b| success(combine(a, b)))
                })
            })
        })
    }
}
impl<I: Send + Sync + 'static, E: Send + 'static, O: Send + Sync + 'static> Layer<I, E, O> {
    pub fn provide_to<A: Send + 'static>(&self, program: Effect<A, E, O>) -> Effect<A, E, I> {
        let layer = self.clone();
        Scope::scoped(move |scope| {
            let build = layer.build(scope);
            let program = program.clone();
            workflow::reusable(move || {
                let program = program.clone();
                bind(build.clone(), move |env| {
                    bind(
                        program.provide(Arc::new(env)).provide_some(|_: &I| ()),
                        success,
                    )
                })
            })
        })
    }
    #[must_use]
    pub fn provide<P: Send + 'static>(self, second: Layer<O, E, P>) -> Layer<I, E, P> {
        Layer::new(move |scope| {
            let build = self.build(scope.clone());
            let second = second.clone();
            workflow::reusable(move || {
                let second = second.clone();
                let scope = scope.clone();
                bind(build.clone(), move |env| {
                    bind(
                        second
                            .build(scope)
                            .provide(Arc::new(env))
                            .provide_some(|_: &I| ()),
                        success,
                    )
                })
            })
        })
    }
}
impl<I: Send + Sync + 'static, E: Send + 'static, T: Send + Sync + 'static>
    Layer<I, E, ResourceValue<T>>
{
    #[must_use]
    pub fn scoped(
        acquire: Effect<T, E, I>,
        release: impl for<'a> Fn(T, crate::PrimaryOutcomeRef<'a, E>) -> Release<E, I>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let release = Arc::new(release);
        Self::new(move |scope| {
            let release = Arc::clone(&release);
            crate::ResourceBinding::new(acquire.clone(), move |value, reason| {
                release(value, reason)
            })
            .acquire_scoped(scope)
        })
    }
}
