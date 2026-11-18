//! The engine's part in the open page's rule table (page-rules.md
//! RULE-58): the calls the decoder's loop makes for it.

use neoteletext_rules::command::{TYPE_ANIMATE, TYPE_NAPLPS_OBJECTS, TYPE_SPRITES};
use neoteletext_web::records::{ChannelRecord, Read};
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::rules::{PageSub, RuleInput};

use super::Engine;
use crate::animation::frames::Taken;
use crate::rules::host::{Parts, PictureCommand};

/// How far right of the 800-pixel picture's left edge a NAPLPS picture
/// begins (RULE-13: a NAPLPS picture is 640 by 480, shown in the middle
/// of the 800 by 480 picture the trusted core gives the pointer in).
const NAPLPS_LEFT: u16 = (interface_core::canvas::REGIS_WIDTH - NAPLPS_WIDTH) / 2;
/// A NAPLPS picture's width in pixels (RULE-13).
const NAPLPS_WIDTH: u16 = neoteletext_web::records::naplps_objects::PICTURE_WIDTH;
const _: () = assert!(NAPLPS_WIDTH as usize == crate::naplps::WIDTH && NAPLPS_LEFT == 80);

/// A pixel of the 800-wide picture as a pixel of the NAPLPS picture in
/// its middle. A pixel of either margin is outside the picture and is
/// given as the first column past its right edge, which no rectangle (cut
/// to the picture, RULE-13) and no object holds: the press, release or
/// move is still followed (RULE-8), on nothing.
const fn naplps_x(x: u16) -> u16 {
    match x.checked_sub(NAPLPS_LEFT) {
        Some(inside) if inside < NAPLPS_WIDTH => inside,
        _ => NAPLPS_WIDTH,
    }
}

/// A rule input of a NAPLPS page, with its pointer in the NAPLPS
/// picture's pixels.
const fn naplps_input(input: RuleInput) -> RuleInput {
    match input {
        RuleInput::Pointer { action, button, x, y, control } => RuleInput::Pointer { action, button, x: naplps_x(x), y, control },
        RuleInput::Wheel { x, y, control } => RuleInput::Wheel { x: naplps_x(x), y, control },
        other => other,
    }
}

/// Reads a picture command's body as the record from the server is read,
/// for the code that applies it (RULE-118).
fn picture_record(kind: u8, body: &[u8]) -> Option<ChannelRecord<'_>> {
    Some(match kind {
        TYPE_ANIMATE => ChannelRecord::Animate(neoteletext_web::records::animate::read(body).ok()?),
        TYPE_SPRITES => ChannelRecord::Sprites(neoteletext_web::records::sprites::read(body).ok()?),
        TYPE_NAPLPS_OBJECTS => ChannelRecord::NaplpsObjects(neoteletext_web::records::naplps_objects::read(body).ok()?),
        _ => return None,
    })
}

/// Runs `$body` with `$parts`, the host's parts for the page `$page`: the
/// audio receiver, the sounds, what is heard and the page's picture.
macro_rules! with_parts {
    ($engine:ident, $page:expr, |$rules:ident, $parts:ident| $body:expr) => {{
        let page: PageSub = $page;
        let facts = $engine.picture_facts_of(page.page, Some(page.sub));
        let Engine { rules: $rules, audio, sounds, heard, regis, naplps, animation, .. } = $engine;
        let mut picture = facts.picture(regis, naplps, animation);
        let mut $parts = Parts { audio: &mut **audio, sounds, heard, picture: &mut picture };
        $body
    }};
}

impl Engine {
    /// The page the host's calls act for: the held table's page, else the
    /// open page, else none.
    fn rules_page(&self) -> PageSub {
        self.rules.held_page().map(|(page, _)| page).or(self.rules.open()).unwrap_or(PageSub::new(neoteletext_rules::limits::FIRST_PAGE, 0))
    }

    /// Takes a message of the open page's rule table (RULE-63): what it
    /// produced for the trusted core, to be written at once, or the
    /// message given back when it is not one.
    ///
    /// # Errors
    /// The message, untouched, when it is not a rule message.
    pub fn rule_message(&mut self, message: ToDecoder) -> Result<Vec<FromDecoder>, ToDecoder> {
        let now_us = self.now_us();
        let produced = match message {
            ToDecoder::RuleInput { page, sequence, input } => self.rule_input(page, sequence, input),
            ToDecoder::Metered { page, values } => {
                self.rules.metered(page, &values);
                Vec::new()
            }
            ToDecoder::RulesQuery { page, identifier } => {
                let mut produced = with_parts!(self, page, |rules, parts| rules.query(page, identifier, now_us, &mut parts));
                produced.extend(self.after_rules());
                produced
            }
            ToDecoder::RulesPage(page) => self.rule_page(page, now_us),
            ToDecoder::ConnectionEnded => self.connection_ended(),
            ToDecoder::CueHeard { audio_channel, number } => {
                let at = self.rules_page();
                let mut produced = with_parts!(self, at, |rules, parts| rules.cue(audio_channel, number, &mut parts));
                produced.extend(self.after_rules());
                produced
            }
            ToDecoder::FramePhase(phase) => {
                self.frame_phase(phase);
                Vec::new()
            }
            other => return Err(other),
        };
        Ok(produced)
    }

    /// The page and sub-code that is the open page of the active tab, or
    /// none (RULE-58, RULE-111): only its table is evaluated.
    fn rule_page(&mut self, page: Option<PageSub>, now_us: u64) -> Vec<FromDecoder> {
        let at = page.unwrap_or_else(|| self.rules_page());
        let mut produced = with_parts!(self, at, |rules, parts| rules.set_open(page, now_us, &mut parts));
        produced.extend(self.after_rules());
        produced
    }

    /// The view's connection ended (RULE-111 case 2).
    fn connection_ended(&mut self) -> Vec<FromDecoder> {
        let at = self.rules_page();
        with_parts!(self, at, |rules, parts| rules.connection_ended(&mut parts));
        // ANIM-2: the connection's end ends every frame.
        self.end_frames();
        let mut produced = self.run_rules(self.now_us());
        produced.extend(self.after_rules());
        produced
    }

    /// An input event of the viewer for the open page, from the trusted
    /// core (RULE-52 item 1), decided at once (RULE-96): what it produced
    /// for the trusted core leaves before any other work.
    fn rule_input(&mut self, page: PageSub, sequence: u32, input: RuleInput) -> Vec<FromDecoder> {
        let facts = self.picture_facts_of(page.page, Some(page.sub));
        let input = if facts.is_naplps(&self.regis, &self.naplps) { naplps_input(input) } else { input };
        let mut produced = with_parts!(self, page, |rules, parts| rules.input(page, sequence, input, &mut parts));
        produced.extend(self.after_rules());
        produced
    }

    /// A command record the server sent for a page (RULE-114): `LOCAL_MIDI`
    /// to its local sound at once, the others to its evaluator.
    pub(super) fn rule_command(&mut self, kind: u8, page: PageSub, body: &[u8]) {
        if kind == neoteletext_web::records::TYPE_LOCAL_MIDI {
            let produced = with_parts!(self, page, |rules, parts| rules.direct_midi(page, body, &mut parts));
            self.rule_out.extend(produced);
        } else {
            let mut produced = with_parts!(self, page, |rules, parts| rules.direct(page, kind, body, &mut parts));
            produced.extend(self.after_rules());
            self.rule_out.extend(produced);
        }
    }

    /// A RULES record (RULE-41), which only this process decodes (RULE-58).
    pub(super) fn rules_record(&mut self, record: &neoteletext_web::records::rules::RulesRecord<'_>) {
        let now_us = self.now_us();
        let page = PageSub::new(record.page.mpp(), record.subcode.value());
        with_parts!(self, page, |rules, parts| rules.record(record, now_us, &mut parts));
        let produced = self.after_rules();
        self.rule_out.extend(produced);
    }

    /// The interface's frames (RULE-98 item 2), read now on the engine's
    /// own clock: the message's time to the next composition is from its
    /// writing, and its time in the pipe the error (DEC-6659 item 12).
    fn frame_phase(&mut self, phase: protocol::rules::FramePhase) {
        let real = self.real_us();
        self.lock.told(phase, real);
    }

    /// When the open page's next tick is due, on [`Engine::now_us`]'s
    /// clock: `None` while no open page has a table, so the decoder keeps
    /// no sixty-a-second wake for rules (RULE-58).
    #[must_use]
    pub fn rules_due_us(&self) -> Option<u64> {
        if !self.rule_out.is_empty() {
            return Some(0);
        }
        self.rules.due_us()
    }

    /// Evaluates each tick of the open page's table that is due at
    /// `now_us` (RULE-4), and gives what they produced for the trusted
    /// core, in the order it is to leave: a group's sound messages first,
    /// before any picture work of the tick (RULE-64).
    pub fn run_rules(&mut self, now_us: u64) -> Vec<FromDecoder> {
        let page = self.rules_page();
        // What the animation's repaints of the last wake cost (ANIM-16):
        // each held back for the budget is told as fault 8 (RULE-48).
        let slowed = self.take_animation_counters_of(page.page, Some(page.sub)).slowed;
        self.rules.animation_slowed(slowed);
        let mut produced = core::mem::take(&mut self.rule_out);
        produced.extend(with_parts!(self, page, |rules, parts| rules.run(now_us, &mut parts)));
        produced.extend(self.after_rules());
        // The frames finished late are the lock's, told with the table's
        // counters (RULE-98 item 5, RULE-72).
        let late_frames = self.lock.late_frames();
        for message in &mut produced {
            if let FromDecoder::RuleCounters(counters) = message {
                counters.late_frames = late_frames;
            }
        }
        produced
    }

    /// What a group did to the picture is applied with the code that
    /// applies the same records from the server (RULE-118, RULE-65), and
    /// what the host dropped of a page is dropped here too (RULE-111).
    /// The voice operations it caused follow.
    fn after_rules(&mut self) -> Vec<FromDecoder> {
        let commands: Vec<PictureCommand> = self.rules.take_picture();
        for (kind, body) in &commands {
            if let Some(taken) = picture_record(*kind, body).map(Read::Record).as_ref().and_then(Taken::of) {
                self.take_animation(taken);
            }
        }
        for page in self.rules.take_dropped() {
            self.drop_page_animation(page.page, page.sub);
        }
        self.sounds.drain(..).map(FromDecoder::Audio).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "tests")]
mod tests;
