//! What a step reports to a channel's scripts of the page the decoder
//! draws (the script API's specification sections 3.5, 4.2, 6.2 and
//! 11.2; TODO-7379 item 3): the cycles that ran once and ended, the
//! bounds of the drawings a script tracks, the sounds that reached a cue
//! or their end, and the animation's faults. The decoder finds each from
//! its own renderers and its own sound state; a batch only says which
//! things a script asked about.

use neoteletext_rules::animate::{Operation, OperationKind};
use neoteletext_rules::limits::AUDIO_CHANNELS;
use neoteletext_rules::table::TargetKind;
use protocol::script::batch::{Batch, MAX_WHEEL_THINGS, SoundOperation};
use protocol::script::{Bounds, FaultReport, MAX_CYCLE_ENDS, MAX_STEP_FAULTS, MAX_STEP_SOUND_REPORTS, MAX_TRACKED_DRAWINGS, SoundReport};

use super::limits::{FAULT_ANIMATION_SKIPPED, FAULT_ANIMATION_SLOWED};
use crate::Engine;
use crate::engine::script::ScriptPage;

/// What a script's sound on a voice is doing, as far as its end goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Voice {
    /// No sound of a script's.
    #[default]
    Idle,
    /// A script asked for a sound that has not been heard to start.
    Asked,
    /// It plays.
    Playing,
}

/// What the decoder watches for a channel's scripts, and what it has
/// found since the last step.
#[derive(Debug, Default)]
pub(super) struct Reports {
    /// The drawings whose bounds a script tracks (the batch's section
    /// WATCH), by handle.
    tracked: Vec<i32>,
    /// The things that take the wheel, by handle.
    wheel: Vec<i32>,
    /// The cycles that run once and have not ended: the thing's handle
    /// and the cycle's last frame.
    once: Vec<(i32, u8)>,
    /// The sound of a script on each voice.
    voices: [Voice; AUDIO_CHANNELS],
    cycle_ended: Vec<i32>,
    sound: Vec<SoundReport>,
    faults: Vec<FaultReport>,
    /// The thing the mouse's pointer is on.
    pub(super) hovered: Option<Hovered>,
}

/// The thing the mouse's pointer is on and where the pointer was when it
/// was last told: the thing's `pointerleave` is given its place within
/// the thing from these.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hovered {
    /// The thing's handle.
    pub(super) thing: i32,
    /// The pointer's place, in the renderer's own units (section 4.4).
    pub(super) at: (i32, i32),
    /// The pointer's place within the thing.
    pub(super) local: (i32, i32),
}

impl Hovered {
    /// Where a pointer now at `at` is within the thing: as far from where
    /// it last was within it as it has moved since. The renderers give
    /// the place within a thing only for the thing under the pointer,
    /// and a pointer that has left is not on it; for a ReGIS drawing
    /// drawn at another scale the step is the screen's, not the
    /// drawing's own.
    pub(super) fn local_of(&self, at: (i32, i32)) -> (i32, i32) {
        let moved = (at.0.saturating_sub(self.at.0), at.1.saturating_sub(self.at.1));
        (self.local.0.saturating_add(moved.0), self.local.1.saturating_add(moved.1))
    }
}

/// What of a step is the reports'.
#[derive(Default)]
pub(super) struct Told {
    pub(super) cycle_ended: Vec<i32>,
    pub(super) bounds: Vec<Bounds>,
    pub(super) sound: Vec<SoundReport>,
    pub(super) faults: Vec<FaultReport>,
}

impl Reports {
    /// The page shown changed or its context ended: everything watched
    /// was the page's. Sounds are the channel's and go on.
    pub(super) fn page_left(&mut self) {
        let voices = self.voices;
        *self = Self { voices, sound: core::mem::take(&mut self.sound), ..Self::default() };
    }

    /// The scripts stopped: nothing is watched any more.
    pub(super) fn forget(&mut self) {
        *self = Self::default();
    }

    /// A batch was applied to the page `at`: what it asks to have watched,
    /// the cycles its operations start and end, and the sounds it starts.
    pub(super) fn applied(&mut self, at: ScriptPage, batch: &Batch) {
        if let Some(watch) = &batch.watch {
            if let Some(tracked) = &watch.tracked {
                self.tracked = tracked.iter().copied().take(MAX_TRACKED_DRAWINGS).collect();
            }
            if let Some(wheel) = &watch.wheel {
                self.wheel = wheel.iter().copied().take(MAX_WHEEL_THINGS).collect();
            }
        }
        // Sprites declared again, and a record of every object, forget
        // every cycle; the script host says each again after it
        // (TODO-7382 step 2(d), TODO-7383 item 2(f)).
        if batch.sprites.is_some() || batch.naplps_objects.as_ref().is_some_and(|cast| !cast.one) {
            self.once.clear();
        }
        for operation in batch.animate.iter().flatten() {
            self.animated(at, operation);
        }
    }

    fn animated(&mut self, at: ScriptPage, operation: &Operation) {
        if operation.target.kind != TargetKind::Object {
            return;
        }
        let handle = Engine::script_handle(at, operation.target.number);
        match operation.kind {
            OperationKind::Cycle { last, once, .. } => {
                self.once.retain(|(cycling, _)| *cycling != handle);
                if once && self.once.len() < MAX_CYCLE_ENDS {
                    self.once.push((handle, last));
                }
            }
            OperationKind::Frame(_) | OperationKind::StopCycle | OperationKind::Reset => self.once.retain(|(cycling, _)| *cycling != handle),
            OperationKind::Show | OperationKind::Hide | OperationKind::Look(_) | OperationKind::Move { .. } | OperationKind::Order { .. } => {}
        }
    }

    /// A batch's sound operations were carried out, whatever page is
    /// shown: a voice a script plays on is listened to for its end.
    pub(super) fn sounded(&mut self, operations: &[SoundOperation]) {
        for operation in operations {
            match *operation {
                SoundOperation::Play { voice, .. } => self.set_voice(voice, Voice::Asked),
                SoundOperation::Stop { voice } => self.set_voice(voice, Voice::Idle),
                SoundOperation::SynthStart { .. } | SoundOperation::SynthStop { .. } | SoundOperation::Midi { .. } => {}
            }
        }
    }

    fn set_voice(&mut self, voice: u8, state: Voice) {
        if let Some(held) = self.voices.get_mut(usize::from(voice)) {
            *held = state;
        }
    }

    /// The sound helper heard a cue of the sound on `voice` (section 3.5).
    /// Only a sound a script started is a script's to hear of.
    pub(super) fn cue(&mut self, voice: u8, cue: u8) -> bool {
        let scripts = self.voices.get(usize::from(voice)).is_some_and(|held| *held != Voice::Idle);
        if scripts && self.sound.len() < MAX_STEP_SOUND_REPORTS {
            self.sound.push(SoundReport::Cue { voice, cue: i32::from(cue) });
        }
        scripts
    }

    /// Whether the decoder must look again at the next tick with nothing
    /// else to wake it: a cycle that runs once, or a script's sound, may
    /// have ended.
    pub(super) fn watching(&self) -> bool {
        !self.once.is_empty() || self.voices.iter().any(|voice| *voice != Voice::Idle)
    }

    /// Looks at the renderers and the sound state for what has happened
    /// since the last look; whether there is something new to tell.
    pub(super) fn gather(&mut self, at: Option<ScriptPage>, engine: &mut Engine) -> bool {
        let mut found = false;
        if let Some(at) = at {
            // A cycle that ran once has ended when its thing shows its
            // last frame; a thing that is gone ends nothing.
            let mut ended = Vec::new();
            self.once.retain(|&(handle, last)| match engine.script_thing(at, handle) {
                Some(state) if state.frame >= last => {
                    ended.push(handle);
                    false
                }
                Some(_) => true,
                None => false,
            });
            for handle in ended {
                if self.cycle_ended.len() < MAX_CYCLE_ENDS {
                    self.cycle_ended.push(handle);
                    found = true;
                }
            }
            let cost = engine.script_animation_cost(at);
            for (count, (code, bound)) in [(cost.slowed, FAULT_ANIMATION_SLOWED), (cost.skipped, FAULT_ANIMATION_SKIPPED)] {
                if count > 0 && self.faults.len() < MAX_STEP_FAULTS && !self.faults.iter().any(|fault| fault.code == code) {
                    self.faults.push(FaultReport { code, bound: bound.to_owned() });
                    found = true;
                }
            }
        }
        for (voice, held) in (0_u8..).zip(self.voices.iter_mut()) {
            let playing = engine.script_voice_playing(voice);
            match (*held, playing) {
                (Voice::Asked, true) => *held = Voice::Playing,
                (Voice::Playing, false) => {
                    *held = Voice::Idle;
                    if self.sound.len() < MAX_STEP_SOUND_REPORTS {
                        self.sound.push(SoundReport::Ended { voice });
                        found = true;
                    }
                }
                _ => {}
            }
        }
        found
    }

    /// Whether anything waits to be told.
    pub(super) fn any(&self) -> bool {
        !(self.cycle_ended.is_empty() && self.sound.is_empty() && self.faults.is_empty())
    }

    /// What the next step tells, with the tracked drawings' bounds as
    /// they stand; a drawing that has none is left out.
    pub(super) fn take(&mut self, at: Option<ScriptPage>, engine: &Engine) -> Told {
        let bounds = at.map_or_else(Vec::new, |at| {
            let of = |handle: &i32| {
                let bounds = engine.script_thing(at, *handle)?.bounds?;
                Some(Bounds {
                    handle: *handle,
                    x: bounds.left,
                    y: bounds.top,
                    width: bounds.right.saturating_sub(bounds.left),
                    height: bounds.bottom.saturating_sub(bounds.top),
                })
            };
            self.tracked.iter().filter_map(of).collect()
        });
        Told { cycle_ended: core::mem::take(&mut self.cycle_ended), bounds, sound: core::mem::take(&mut self.sound), faults: core::mem::take(&mut self.faults) }
    }

    /// The thing of `things`, topmost first, that takes the wheel, if
    /// one does (section 4.2: "Outside a thing created with `wheel: true`
    /// the wheel is the viewer's and no event is sent").
    pub(super) fn wheel_thing(&self, things: &[i32]) -> Option<i32> {
        things.iter().copied().find(|thing| self.wheel.contains(thing))
    }
}
