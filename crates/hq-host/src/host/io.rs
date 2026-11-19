//! Talking to a running pane: answering dialogs, reading, sending input, waiting and killing.

use super::*;

impl Host {
    /// Answers the dialog numbered `seq`: picks option `id`, or declines it when `id` is `None`.
    /// Refused when the agent is at another dialog by now. Arrow keys move the cursor and Enter
    /// is pressed only once the screen shows the cursor on that option. Then waits a moment for
    /// the dialog to go away; `registered` is false when it is still there.
    pub fn answer(&self, name: &str, seq: u64, id: Option<u32>) -> Result<Answered, HostError> {
        if let Some(acp) = self.acp_pane(name) {
            return self.answer_acp(name, &acp, seq, id);
        }
        self.live_pane(name)?;
        let info = self.announce(name)?;
        let (Some(dialog), Some(current)) = (info.pending.clone(), info.pending_seq) else {
            return Err(HostError::NoDialog(name.to_string()));
        };
        if current != seq {
            return Err(HostError::StaleDialog {
                name: name.to_string(),
                current,
            });
        }
        match id {
            None => self.send_keys(name, &dialog.decline)?,
            Some(id) => self.pick(name, &dialog, id)?,
        }
        let deadline = Instant::now() + ANSWER_SETTLE;
        loop {
            std::thread::sleep(STATE_POLL);
            let after = self.announce(name)?;
            let gone = after.state != Some(AgentState::Blocked)
                || after.pending_seq.is_some_and(|s| s != seq);
            if gone || Instant::now() >= deadline {
                return Ok(Answered {
                    registered: gone,
                    info: after,
                });
            }
        }
    }

    fn answer_acp(
        &self,
        name: &str,
        acp: &AcpPane,
        seq: u64,
        id: Option<u32>,
    ) -> Result<Answered, HostError> {
        acp.answer(name, seq, id)?;
        let deadline = Instant::now() + ANSWER_SETTLE;
        loop {
            std::thread::sleep(STATE_POLL);
            let after = acp.info(name);
            let gone = after.pending_seq != Some(seq);
            if gone || Instant::now() >= deadline {
                return Ok(Answered {
                    registered: gone,
                    info: after,
                });
            }
        }
    }

    fn pick(&self, name: &str, dialog: &Dialog, id: u32) -> Result<(), HostError> {
        let target = dialog.option(id).ok_or(HostError::NoSuchOption(id))?;
        if target.typed {
            return Err(HostError::TypedOption(id));
        }
        if dialog.answer == AnswerMethod::Digit && (1..=9).contains(&id) {
            return self.send_keys(name, &[id.to_string()]);
        }
        let place = |id: u32| dialog.options.iter().position(|o| o.id == id);
        let from = dialog.selected().ok_or(HostError::NoCursor)?;
        let (from, to) = (place(from.id), place(id));
        let (Some(from), Some(to)) = (from, to) else {
            return Err(HostError::NoCursor);
        };
        let key = if to > from { "down" } else { "up" };
        for _ in 0..from.abs_diff(to) {
            // One key per write: a TUI can read several escape sequences in one chunk as one key.
            self.send_keys(name, &[key.to_string()])?;
            std::thread::sleep(ARROW_GAP);
        }
        // Enter picks whatever is highlighted, so it waits until the screen shows the right one.
        let deadline = Instant::now() + CURSOR_WAIT;
        loop {
            let now = self.announce(name)?;
            let there = now.pending.as_deref().is_some_and(|d| {
                d.identity() == dialog.identity()
                    && d.selected()
                        .is_some_and(|o| o.id == id && o.label == target.label)
            });
            if there {
                return self.send_keys(name, &["enter".to_string()]);
            }
            if Instant::now() >= deadline {
                return Err(HostError::AnswerUnverified(id));
            }
            std::thread::sleep(STATE_POLL);
        }
    }

    /// Forgets an agent and stops its process and the processes it started.
    pub fn remove(&self, name: &str) -> Result<(), HostError> {
        // Taken out first: `save` locks the registry again.
        let removed = lock(&self.acp).remove(name);
        if let Some(acp) = removed {
            acp.kill();
            self.save();
            self.forget_files(name);
            self.events.push(name, EventKind::Removed, None, None);
            return Ok(());
        }
        let Some(pane) = lock(&self.panes).remove(name) else {
            if lock(&self.awaiting).remove(name).is_some() {
                self.forget_files(name);
                self.save();
                self.events.push(name, EventKind::Removed, None, None);
                return Ok(());
            }
            return Err(HostError::NotFound(name.to_string()));
        };
        lock(&self.pane_tokens).remove(name);
        self.save();
        // Other threads may still hold the pane (a caller waiting on it), so
        // stopping it cannot be left to the last reference being dropped.
        pane.kill();
        self.forget_files(name);
        self.events.push(name, EventKind::Removed, None, None);
        Ok(())
    }

    /// The last `lines` lines of the chosen view (all of them when `lines` is 0).
    pub fn read(&self, name: &str, source: ReadSource, lines: usize) -> Result<String, HostError> {
        if let Some(acp) = self.acp_pane(name) {
            let (mut all, partial, _) = acp.transcript();
            all.extend(Some(partial).filter(|p| !p.is_empty()));
            if lines > 0 && all.len() > lines {
                all.drain(..all.len() - lines);
            }
            return Ok(all.join("\n"));
        }
        let pane = self.pane(name)?;
        let mut out: Vec<String> = match source {
            ReadSource::Visible => pane.rows(false).into_iter().map(|r| r.text).collect(),
            ReadSource::Recent => pane.rows(true).into_iter().map(|r| r.text).collect(),
            ReadSource::RecentUnwrapped => unwrap_rows(pane.rows(true)),
            ReadSource::Styled => pane.with_emu(|e| e.styled_history(lines)),
        };
        if lines > 0 && out.len() > lines {
            out.drain(..out.len() - lines);
        }
        Ok(out.join("\n"))
    }

    /// Output that scrolled off the screen since `cursor`, plus the screen as it is now and the
    /// cursor for next time. Only scrollback is final: a program redraws its screen in place, so
    /// the screen comes whole every time. `cursor` is opaque; `None` starts one. When it cannot
    /// be placed (the rows it marks were dropped or never existed) the reply says `reset` and
    /// holds the newest lines instead. At most `max` lines come back, the newest.
    pub fn read_since(
        &self,
        name: &str,
        cursor: Option<&str>,
        max: usize,
    ) -> Result<ReadSince, HostError> {
        if let Some(acp) = self.acp_pane(name) {
            let (done, partial, _) = acp.transcript();
            let (from, reset) = match cursor {
                None => (done.len(), false),
                Some(c) => match place_cursor(&done, c) {
                    Some(at) => (at, false),
                    None => (done.len().saturating_sub(max), true),
                },
            };
            let new = &done[from.min(done.len())..];
            let skipped = new.len().saturating_sub(max);
            return Ok(ReadSince {
                lines: new[skipped..].to_vec(),
                skipped,
                screen: Some(partial)
                    .filter(|p| !p.is_empty())
                    .into_iter()
                    .collect(),
                next: cursor_at(&done),
                reset,
            });
        }
        let pane = self.pane(name)?;
        let (history, visible) = pane.with_emu(|e| (e.history(), e.visible()));
        let off_screen = history.len().saturating_sub(visible.len());
        let scrolled = unwrap_rows(history.into_iter().take(off_screen).collect());
        let screen = unwrap_rows(visible);
        let (from, reset) = match cursor {
            None => (scrolled.len(), false),
            Some(c) => match place_cursor(&scrolled, c) {
                Some(at) => (at, false),
                None => (scrolled.len().saturating_sub(max), true),
            },
        };
        let new = &scrolled[from.min(scrolled.len())..];
        let skipped = new.len().saturating_sub(max);
        Ok(ReadSince {
            lines: new[skipped..].to_vec(),
            skipped,
            screen,
            next: cursor_at(&scrolled),
            reset,
        })
    }

    fn live_pane(&self, name: &str) -> Result<Arc<Pane>, HostError> {
        let pane = self.pane(name)?;
        if pane.status() != PaneStatus::Running {
            return Err(HostError::Exited(name.to_string()));
        }
        Ok(pane)
    }

    /// Raw bytes, exactly as given.
    pub fn send_text(&self, name: &str, text: &str) -> Result<(), HostError> {
        if self.acp_pane(name).is_some() {
            return Err(HostError::Unsupported(RAW_INPUT_UNSUPPORTED));
        }
        self.live_pane(name)?.write(text.as_bytes())
    }

    /// Text as a paste: bracketed when the program asked for it, so embedded
    /// newlines are not taken for Enter.
    pub fn paste(&self, name: &str, text: &str) -> Result<(), HostError> {
        if self.acp_pane(name).is_some() {
            return Err(HostError::Unsupported(RAW_INPUT_UNSUPPORTED));
        }
        let pane = self.live_pane(name)?;
        if pane.with_emu(|e| e.bracketed_paste()) {
            // The text cannot end the paste early: whatever followed a forged
            // end marker would be typed as keystrokes.
            let mut bytes = BRACKETED_PASTE_START.to_vec();
            bytes.extend_from_slice(without_paste_markers(text).as_bytes());
            bytes.extend_from_slice(BRACKETED_PASTE_END);
            pane.write(&bytes)
        } else {
            pane.write(text.as_bytes())
        }
    }

    /// Logical key names (`enter`, `esc`, `ctrl+c`, ...), validated before anything is sent.
    pub fn send_keys(&self, name: &str, keys: &[String]) -> Result<(), HostError> {
        if let Some(acp) = self.acp_pane(name) {
            // The only keys that mean something to a protocol agent are the ones that stop a turn.
            let stops =
                !keys.is_empty() && keys.iter().all(|k| matches!(k.as_str(), "esc" | "ctrl+c"));
            if !stops {
                return Err(HostError::Unsupported(RAW_INPUT_UNSUPPORTED));
            }
            return acp.cancel(name);
        }
        let mut bytes = Vec::new();
        for key in keys {
            bytes.extend(encode_key(key)?);
        }
        self.live_pane(name)?.write(&bytes)
    }

    /// Pastes `text` and presses Enter, unless the agent is waiting at a dialog: there the
    /// Enter would pick whichever option is highlighted, which can be one that widens what the
    /// agent may do ("don't ask again"). A dialog is answered with keys, on purpose.
    pub fn prompt(&self, name: &str, text: &str) -> Result<(), HostError> {
        if let Some(acp) = self.acp_pane(name) {
            return acp.prompt(name, text);
        }
        let pane = self.live_pane(name)?;
        if info_of(name, &pane, &self.detector).state == Some(AgentState::Blocked) {
            return Err(HostError::Blocked(name.to_string()));
        }
        self.paste(name, text)?;
        std::thread::sleep(PROMPT_ENTER_DELAY);
        self.send_keys(name, &["enter".to_string()])
    }

    pub fn resize(&self, name: &str, rows: u16, cols: u16) -> Result<(), HostError> {
        check_size(rows, cols)?;
        if self.acp_pane(name).is_some() {
            return Ok(());
        }
        self.live_pane(name)?.resize(rows, cols)
    }

    pub fn wait_exit(&self, name: &str, timeout: Duration) -> Result<u32, HostError> {
        if let Some(acp) = self.acp_pane(name) {
            let deadline = Instant::now() + timeout;
            loop {
                if let PaneStatus::Exited { code } = acp.status() {
                    return Ok(code);
                }
                if Instant::now() >= deadline {
                    return Err(HostError::Timeout(timeout));
                }
                std::thread::sleep(STATE_POLL);
            }
        }
        self.pane(name)?.wait_exit(timeout)
    }

    pub fn wait_quiet(
        &self,
        name: &str,
        quiet: Duration,
        timeout: Duration,
    ) -> Result<(), HostError> {
        if let Some(acp) = self.acp_pane(name) {
            let deadline = Instant::now() + timeout;
            while acp.quiet_for() < quiet {
                if Instant::now() >= deadline {
                    return Err(HostError::Timeout(timeout));
                }
                std::thread::sleep(STATE_POLL);
            }
            return Ok(());
        }
        self.pane(name)?.wait_quiet(quiet, timeout)
    }

    /// Waits until the detected state is one of `states` and has held for
    /// `stable`, so a flicker between two screens is not taken for a change.
    /// An agent kind without a rule file never matches and times out.
    pub fn wait_state(
        &self,
        name: &str,
        states: &[AgentState],
        stable: Duration,
        timeout: Duration,
    ) -> Result<PaneInfo, HostError> {
        let deadline = Instant::now() + timeout;
        let mut since: Option<(AgentState, Instant)> = None;
        loop {
            let info = self.info(name)?;
            match info.state {
                Some(state) if states.contains(&state) => {
                    let held = match since {
                        Some((s, at)) if s == state => at,
                        _ => Instant::now(),
                    };
                    since = Some((state, held));
                    if held.elapsed() >= stable {
                        return Ok(info);
                    }
                }
                _ => since = None,
            }
            let over = info.state.is_none() || self.acp_pane(name).is_some();
            if info.status != PaneStatus::Running && over {
                return Err(HostError::Exited(name.to_string()));
            }
            if Instant::now() >= deadline {
                return Err(HostError::Timeout(timeout));
            }
            std::thread::sleep(STATE_POLL);
        }
    }

    pub fn kill(&self, name: &str) -> Result<(), HostError> {
        if let Some(acp) = self.acp_pane(name) {
            acp.kill();
            // Do not wait for the exit: a host that dies now must not bring it back.
            self.save();
            return Ok(());
        }
        self.pane(name)?.kill();
        // Do not wait for the exit: a host that dies now must not bring it back.
        self.save();
        Ok(())
    }
}
