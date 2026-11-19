//! A physically reserved, one-shot route input shared by publisher and factory.
use super::*;
use crate::memory::{RetainedFrameValue, SharedRuntimeFrame};

mod sealed {
    use super::*;
    pub trait Sealed<D> {}
    impl<F, D> Sealed<D> for F where F: Fn() -> D {}
    impl<F, I, D> Sealed<D> for InputRecipe<F, I> where F: Fn(I) -> D {}
}
/// Internal recipe strategy shared by the ordinary and late-input root factory.
#[doc(hidden)]
pub trait RootRecipe<D: Description>: sealed::Sealed<D> + Send + Sync + 'static {
    type Held: Send + 'static;
    type Publisher;
    fn input_demand() -> Option<Demand>;
    fn reserve_input(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<D::Value, D::Error>,
    ) -> Result<(Self::Publisher, Self::Held), MemoryError>;
    fn invoke(&self, input: Self::Held) -> Result<D, Defect>;
}
impl<F, D> RootRecipe<D> for F
where
    D: Description,
    F: Fn() -> D + Send + Sync + 'static,
{
    type Held = ();
    type Publisher = ();
    fn input_demand() -> Option<Demand> {
        None
    }
    fn reserve_input(
        _: &FramePool<'static>,
        _: &PreparedRunContext<D::Value, D::Error>,
    ) -> Result<((), ()), MemoryError> {
        Ok(((), ()))
    }
    fn invoke(&self, (): ()) -> Result<D, Defect> {
        Ok(self())
    }
}
/// Shared prepared recipe which consumes one published input per run.
pub struct InputRecipe<F, I> {
    pub(super) factory: F,
    pub(super) marker: PhantomData<fn(I)>,
}
impl<F, I, D> RootRecipe<D> for InputRecipe<F, I>
where
    D: Description,
    I: Send + 'static,
    F: Fn(I) -> D + Send + Sync + 'static,
{
    type Held = Input<I, D::Value, D::Error>;
    type Publisher = InputPublisher<I, D::Value, D::Error>;
    fn input_demand() -> Option<Demand> {
        Some(Input::<I, D::Value, D::Error>::demand())
    }
    fn reserve_input(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<D::Value, D::Error>,
    ) -> Result<(Self::Publisher, Self::Held), MemoryError> {
        Input::reserve(frames, context)
    }
    fn invoke(&self, input: Self::Held) -> Result<D, Defect> {
        input
            .take()
            .map(&self.factory)
            .ok_or_else(|| Defect::from(crate::LibraryDefect::MissingInterpreterOutcome))
    }
}

type Record<I, A, E> = RetainedFrameValue<Mutex<Option<I>>, PreparedRunContext<A, E>>;

pub struct Input<I: Send + 'static, A: Send + 'static, E: Send + 'static> {
    frame: SharedRuntimeFrame,
    #[allow(clippy::type_complexity)]
    marker: PhantomData<fn() -> (I, A, E)>,
}
pub struct InputPublisher<I: Send + 'static, A: Send + 'static, E: Send + 'static> {
    input: Input<I, A, E>,
}
impl<I: Send + 'static, A: Send + 'static, E: Send + 'static> Input<I, A, E> {
    pub(super) fn demand() -> Demand {
        Demand::cell::<Record<I, A, E>>(false)
    }
    pub(super) fn reserve(
        frames: &FramePool<'static>,
        context: &PreparedRunContext<A, E>,
    ) -> Result<(InputPublisher<I, A, E>, Self), MemoryError> {
        let frame = frames
            .reserve::<Record<I, A, E>>()?
            .share_parts(Mutex::new(None), context.try_clone()?);
        let input = Self {
            frame: frame.try_clone()?,
            marker: PhantomData,
        };
        Ok((
            InputPublisher {
                input: Self {
                    frame,
                    marker: PhantomData,
                },
            },
            input,
        ))
    }
    fn cell(&self) -> &Mutex<Option<I>> {
        // SAFETY: the private constructor pairs this exact record type and frame.
        unsafe { Record::<I, A, E>::value_pointer(self.frame.pointer().cast()).as_ref() }
    }
    pub(super) fn take(&self) -> Option<I> {
        self.cell()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}
impl<I: Send + 'static, A: Send + 'static, E: Send + 'static> InputPublisher<I, A, E> {
    /// Publish the uniquely owned decoded input once, before the driver is polled.
    pub fn publish(self, value: I) {
        let previous = self
            .input
            .cell()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(value);
        drop(previous);
    }
}
