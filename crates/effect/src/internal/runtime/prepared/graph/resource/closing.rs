//! Reuse lexical body/outcome/terminal handoff for a shared Layer scope.
use super::*;
use crate::PreparedScopeOwner;
pub(in crate::internal::runtime::prepared::graph) struct Closing<N: Description, V: Send + 'static>
{
    finished: FrameReservation<'static, FinishBody<N, V>>,
    slot: Slot<Exit<N::Value, N::Error>>,
    close: PreparedScopeClose<(), N::Error, V>,
    after: FrameReservation<'static, AfterClose<N, V>>,
    terminal: FrameReservation<'static, Terminal<N, V>>,
    context: PreparedRunContext<V, N::Error>,
    cleanup_buffer: crate::CleanupFailures<N::Error>,
}
impl<N: Description, V: Send + 'static> Closing<N, V> {
    pub(in crate::internal::runtime::prepared::graph) fn shape(
        finalizers: usize,
    ) -> Result<Demand, MemoryError> {
        let close = PreparedScopeClose::<(), N::Error, V>::demand(finalizers)?;
        Demand {
            frame_layout: close.frame_layout,
            frames_per_run: close.frames,
            continuations: 0,
            descriptions: 0,
            origin_frames: 0,
            environment_origin_frames: 0,
            record_payload: Bytes::new(0),
            cleanup_sites: close
                .cleanup_sites
                .checked_add(1)
                .ok_or(MemoryError::ArithmeticOverflow)?,
            panic_sites: close.panic_sites,
        }
        .add(Environment::<Mutex<Option<Exit<N::Value, N::Error>>>>::demand::<V, N::Error>())?
        .add(Demand::cell::<FinishBody<N, V>>(true))?
        .add(Demand::cell::<AfterClose<N, V>>(true))?
        .add(Demand::cell::<Terminal<N, V>>(true))
    }
    pub(in crate::internal::runtime::prepared::graph) fn reserve(
        owner: PreparedScopeOwner<N::Error>,
        frames: &FramePool<'static>,
        context: &PreparedRunContext<V, N::Error>,
        protected: &PreparedRunContext<V, N::Error>,
    ) -> Result<Self, MemoryError> {
        let cleanup_buffer = owner.reserve_cleanup_buffer()?;
        let close = crate::effect_scope! { reserve_close; owner: owner,frames: frames,context: protected, }?;
        Ok(Self {
            finished: frames.reserve()?,
            slot: Slot::prepare(frames, context)?,
            close,
            after: frames.reserve()?,
            terminal: frames.reserve()?,
            context: context.try_clone()?,
            cleanup_buffer,
        })
    }
    pub(in crate::internal::runtime::prepared::graph) fn continuation(
        self,
        finish: Finish<N::Value, N::Error>,
    ) -> Finish<N::Value, N::Error> {
        Once::prepared(
            self.finished,
            FinishBody {
                slot: self.slot,
                close: self.close,
                after: self.after,
                terminal: self.terminal,
                finish,
                context: self.context,
                cleanup_buffer: self.cleanup_buffer,
            },
        )
    }
}
