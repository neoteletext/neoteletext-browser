//! Typing into a page (`interaction.md`, DOC-1781): the keyboard's intents
//! for the open page's channel, and what the typing rules decided,
//! delivered.

use protocol::{ToCore, ToInterface};

use super::Core;

impl Core {
    /// PICK for a pointer event on a page drawn in ReGIS: the numbered
    /// drawing under the pointer, from the pick record of the picture the
    /// interface was last sent, kept with the event just given so that it
    /// follows it in the event's report (page-rules.md RULE-46). Nothing
    /// for any other page or event.
    #[cfg(feature = "interaction")]
    fn pick_under(&mut self, input: &protocol::interaction::InputIntent) {
        use protocol::interaction::Intent;
        if !matches!(input.intent, Intent::Mouse { .. } | Intent::Touch { .. } | Intent::Wheel { .. }) {
            return;
        }
        // The place is the picture's own pixel, as the interface said it
        // before this event (REQ-6377), not the event's dot of the page image.
        let (Some(pixel), Some(on)) = (self.regis_pointing.at(input.key), self.typing.on_screen(input.key.channel)) else { return };
        let Some(under) = self.regis.under(input.key, on.sub, pixel) else { return };
        let pick = neoteletext_input::event::Pick {
            on,
            drawing: under.drawing,
            x: under.x,
            y: under.y,
            picture_x: under.picture_x,
            picture_y: under.picture_y,
            commits: under.commits,
        };
        self.typing.pick(input.key, pick);
    }

    /// What the trusted core derives for a pointer intent before the
    /// typing rules take it (RULE-52 item 1): the pixel of the page's
    /// picture, for a page a renderer other than teletext's draws, and
    /// the teletext control under its cell.
    #[cfg(feature = "interaction")]
    fn pointer_context(&mut self, input: &protocol::interaction::InputIntent) {
        use protocol::interaction::Intent;
        let cell = match &input.intent {
            Intent::Mouse { at, .. } | Intent::Touch { at, .. } | Intent::Wheel { at, .. } => u8::try_from(at.column).ok().zip(u8::try_from(at.row).ok()),
            Intent::Hover { column, row, .. } => Some((*column, *row)),
            _ => return,
        };
        #[cfg(feature = "links")]
        let control = cell.map_or(0, |(column, row)| self.links.control_at(input.key, column, row));
        #[cfg(not(feature = "links"))]
        let control = {
            let _ = cell;
            0
        };
        self.typing.pointer_context(input.key.channel, self.regis_pointing.at(input.key), control);
    }

    /// The keyboard's intents: input for the open page's channel, and the
    /// phone keyboard's panel.
    #[cfg(feature = "interaction")]
    pub(super) fn input_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Input(intent) => {
                // RULE-66's hops: when the trusted core took the input.
                #[cfg(feature = "timed-test")]
                protocol::timed_test::mark("core", "input-read", std::time::Instant::now(), format_args!(""));
                // A HOVER names a control of the page's latest image, or
                // goes nowhere (REQ-INT-121).
                #[cfg(feature = "links")]
                if let protocol::interaction::Intent::Hover { column, row, .. } = &intent.intent
                    && !self.links.is_control_at(intent.key, *column, *row)
                {
                    return Ok(());
                }
                // A script's page is given the viewer's input by the gate
                // and by nothing else (script-api.md section 4.6).
                if self.script_input(&intent)? {
                    return Ok(());
                }
                self.pointer_context(&intent);
                let sends = self.typing.input_intent(&intent, self.now_us());
                // What was given to the page's decoder (RULE-52 item 1).
                if sends.iter().any(|send| matches!(send, neoteletext::typing::Send::Decoder { .. })) {
                    // On a page drawn in ReGIS, a pointer event is followed
                    // by what lay under it (PICK, the engine's REQ-6337).
                    self.pick_under(&intent);
                    // It is answered by the page's next frame, which its
                    // interval does not hold (REQ-RCH-002, RULE-65).
                    self.frames.input(intent.key);
                    if intent.intent.is_activation() {
                        // The viewer acted: a channel may now ask to know
                        // them (VS-104, VS-212).
                        self.viewer_gesture(intent.key.channel);
                        self.session.viewer_input(intent.key.channel);
                    }
                }
                self.deliver_typing(sends)?;
            }
            ToCore::Editing(editing) => {
                let sends = self.typing.editing(editing, self.now_us());
                self.deliver_typing(sends)?;
            }
            ToCore::StopScripts => self.script_stop_asked(),
            ToCore::WallWithoutPage => {
                if let Some(channel) = self.active_channel() {
                    let sends = self.typing.selects_nothing(channel, self.now_us());
                    self.deliver_typing(sends)?;
                }
            }
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(open) => self.phone_panel(open)?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePin => self.phone_with_pin()?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhoneFinish => {
                if let Some((_, input)) = &mut self.phone {
                    let _ = protocol::keyboard::write_to_keyboard(input, &protocol::keyboard::ToKeyboard::Finish);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Delivers what the typing rules decided: input messages to a
    /// channel's fetcher, each move tagged with its contact and the
    /// messages of one moment marked to leave together; the round trip's
    /// measurement on or off; the typing state to the interface.
    #[cfg(feature = "interaction")]
    pub(super) fn deliver_typing(&mut self, sends: Vec<neoteletext::typing::Send>) -> Result<(), String> {
        use neoteletext::typing::Send;
        use protocol::fetcher::{ToFetcher, write_to_fetcher};
        for send in sends {
            match send {
                Send::Fetcher { channel, bytes, move_of, more } => {
                    // The trace reads the message as it goes (`input_trace.rs`).
                    let line = self.input_trace.as_mut().map(|trace| trace.note(&bytes));
                    self.record_sent_bytes(channel, &bytes);
                    let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) else { continue };
                    let length = bytes.len();
                    // A fetcher that cannot be written to has stopped; its
                    // reader reports that, and the socket's state follows.
                    if write_to_fetcher(input, &ToFetcher::Send { bytes, move_of, more }).is_ok() {
                        self.console.sent(channel, length);
                        if let Some(line) = line {
                            self.send(&ToInterface::InputTrace(line))?;
                        }
                    }
                }
                Send::Measure { channel, on } => {
                    if let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) {
                        let _ = write_to_fetcher(input, &ToFetcher::MeasureRoundTrip(on));
                    }
                }
                // An input event for the open page's decoder, whose table
                // asked for its class (RULE-52 item 1).
                Send::Decoder { channel, page, sequence, input, .. } => {
                    // RULE-66's hops: when the event was given the decoder's pipe.
                    #[cfg(feature = "timed-test")]
                    {
                        protocol::timed_test::mark("core", "input-told", std::time::Instant::now(), format_args!("{sequence}"));
                        protocol::timed_test::owe_flush();
                    }
                    self.tell(channel, protocol::decoder::ToDecoder::RuleInput { page, sequence, input });
                }
                // The open page for its table, or the pause (RULE-111 case
                // 4): the view's local sounds are silenced, their
                // instruments kept, while it is paused.
                Send::RulesPage { channel, page } => {
                    // The scripts of the page left hear `navigate` first
                    // (script-api.md section 4.2).
                    #[cfg(feature = "links")]
                    if let Some(next) = page {
                        self.script_page_leaving(channel, next);
                    }
                    // Whether the tab is shown is told apart from which
                    // page is open: no page is open between two pages of
                    // a tab in front, which hides nothing, and only a
                    // hidden tab pauses a channel's scripts (script-api.md
                    // section 3.1; TODO-7405).
                    let shown = page.is_some() || self.active_channel() == Some(channel);
                    self.tell(channel, protocol::decoder::ToDecoder::Script(protocol::decoder::script::ScriptOrder::TabShown(shown)));
                    self.tell(channel, protocol::decoder::ToDecoder::RulesPage(page));
                    // A page opening is told the interface's frames again,
                    // for its tick 0 (RULE-98 item 2).
                    if page.is_some() {
                        self.tell_frame_phase(channel);
                    }
                    self.tell_sound(&protocol::sound::ToSound::Audio { channel, audio: protocol::audio::Audio::Pause(page.is_none()) });
                }
                Send::ConnectionEnded { channel } => self.tell(channel, protocol::decoder::ToDecoder::ConnectionEnded),
                Send::Interface(state) => {
                    // What the open page's scripts hold is the page's, as
                    // a table's keys are (script-api.md section 4.6).
                    let state = self.script_typing(state);
                    self.send(&ToInterface::Typing(state))?;
                    // Keys may have started or stopped going to the
                    // channel: the looks and the hover follow (REQ-6374).
                    self.regis_repoint()?;
                }
            }
        }
        Ok(())
    }

    /// When the pointer's next waiting move falls due (REQ-RCH-032): the
    /// trusted core's timer for it, none while nothing waits.
    #[cfg(feature = "interaction")]
    pub(super) fn moves_due_at(&self) -> Option<std::time::Instant> {
        self.typing.due().and_then(|due| self.started.checked_add(std::time::Duration::from_micros(due)))
    }

    /// Writes the moves whose period has ended (REQ-RCH-031).
    #[cfg(feature = "interaction")]
    pub(super) fn send_due_moves(&mut self) -> Result<(), String> {
        let sends = self.typing.take_due(self.now_us());
        self.deliver_typing(sends)
    }
}
