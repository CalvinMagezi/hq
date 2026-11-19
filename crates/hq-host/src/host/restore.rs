//! Starting again after a restart: restore, resume and shutdown.

use super::*;

impl Host {
    /// Starts again every agent the state file lists, using its resume
    /// command. Call once, before serving. Under lockdown nothing starts: every
    /// record waits, so it is kept and can be resumed once the host is unlocked.
    pub fn restore(&self) -> RestoreReport {
        let mut report = RestoreReport::default();
        let Some(state) = &self.state else {
            return report;
        };
        let records = match state.read() {
            Ok(records) => records,
            Err(e) => {
                report
                    .skipped
                    .push(("session.json".to_string(), e.to_string()));
                return report;
            }
        };
        // Nothing is written until every record has been dealt with, so a crash halfway leaves
        // the file as it was and the records not yet started are still there at the next start.
        state.hold();
        let mut locked = None;
        let mut to_start = Vec::new();
        for rec in records {
            // Read each time: `hq host lockdown` may land while this runs.
            locked = locked.or_else(|| self.lockdown_status());
            if let Some(held) = &locked {
                report.skipped.push((
                    rec.name.clone(),
                    format!(
                        "the host is locked down, so it is forgotten: {}",
                        held.reason
                    ),
                ));
            } else if !rec.cwd.is_dir() {
                // A finished session whose worktree was removed would otherwise start in a
                // folder that is gone, exit at once and be saved again at every restart.
                report.skipped.push((
                    rec.name.clone(),
                    format!(
                        "its folder {} is gone, so it is forgotten",
                        rec.cwd.display()
                    ),
                ));
            } else if !rec.env_keys.is_empty() {
                lock(&self.awaiting).insert(rec.name.clone(), rec);
            } else {
                to_start.push(rec);
            }
        }
        for (rec, started) in self.start_records(to_start) {
            let name = rec.name.clone();
            // Lockdown may have landed while the starts ran, after the check above.
            let started = started.and_then(|info| self.undo_if_locked(&info.name).map(|()| info));
            match started {
                Ok(_) => report.restored.push(name),
                Err(HostError::LockedDown(reason)) => report.skipped.push((
                    name,
                    format!("the host is locked down, so it is forgotten: {reason}"),
                )),
                // Over the cap the record waits instead of being lost at the next save.
                Err(e @ HostError::TooManyAgents(_)) => {
                    report.skipped.push((name, e.to_string()));
                    lock(&self.awaiting).insert(rec.name.clone(), rec);
                }
                // An ACP agent that would not load (a login that ran out, a busy machine) keeps its
                // record, so resuming it later still finds the session.
                Err(e) if rec.acp_session.is_some() => {
                    report
                        .skipped
                        .push((name, format!("{e} (kept: resume it once that is fixed)")));
                    lock(&self.awaiting).insert(rec.name.clone(), rec);
                }
                Err(e) => report.skipped.push((name, e.to_string())),
            }
        }
        state.release();
        self.save();
        self.sweep_agent_files();
        report
    }

    /// Starts the records: terminal agents one by one (a start is quick), ACP agents a few at a
    /// time because each waits on its server to initialize and load its session.
    fn start_records(
        &self,
        records: Vec<PaneRecord>,
    ) -> Vec<(PaneRecord, Result<PaneInfo, HostError>)> {
        let (acp, terminal): (Vec<_>, Vec<_>) =
            records.into_iter().partition(|r| r.acp_session.is_some());
        let mut done: Vec<_> = terminal
            .into_iter()
            .map(|rec| {
                let started = self.start(spec_of(rec.clone(), Vec::new()));
                (rec, started)
            })
            .collect();
        let mut waiting = acp.as_slice();
        while !waiting.is_empty() {
            // A start counts toward the cap only once it has launched, so a batch never holds
            // more than the room left.
            let room = self.max_agents.saturating_sub(self.agent_count());
            let take = room.clamp(1, MAX_PARALLEL_ACP_RESTORES).min(waiting.len());
            let (batch, rest) = waiting.split_at(take);
            waiting = rest;
            std::thread::scope(|scope| {
                let running: Vec<_> = batch
                    .iter()
                    .map(|rec| scope.spawn(|| self.start(spec_of(rec.clone(), Vec::new()))))
                    .collect();
                for (rec, handle) in batch.iter().zip(running) {
                    let started = handle.join().unwrap_or_else(|_| {
                        Err(HostError::Spawn {
                            command: rec.name.clone(),
                            detail: "the restore thread panicked".to_string(),
                        })
                    });
                    done.push((rec.clone(), started));
                }
            });
        }
        done
    }

    /// Agents restored from the state file that wait for their environment.
    pub fn awaiting(&self) -> Vec<AwaitingInfo> {
        lock(&self.awaiting)
            .values()
            .map(|r| AwaitingInfo {
                name: r.name.clone(),
                agent: r.agent.clone(),
                cwd: r.cwd.clone(),
                env_keys: r.env_keys.clone(),
            })
            .collect()
    }

    /// Starts a waiting agent with the environment its starter supplies again.
    /// It stays waiting if a variable is missing or the start fails.
    pub fn resume(&self, name: &str, env: Vec<(String, String)>) -> Result<PaneInfo, HostError> {
        self.resume_with(name, env, None)
    }

    /// `resume`, also giving an ACP agent HQ's MCP connection in its session. The endpoint is never
    /// stored, so a restored agent has none until the caller supplies it here.
    pub fn resume_with(
        &self,
        name: &str,
        env: Vec<(String, String)>,
        mcp: Option<McpEndpoint>,
    ) -> Result<PaneInfo, HostError> {
        let rec = lock(&self.awaiting)
            .get(name)
            .cloned()
            .ok_or_else(|| HostError::NotFound(name.to_string()))?;
        let missing: Vec<&str> = rec
            .env_keys
            .iter()
            .filter(|k| !env.iter().any(|(have, _)| have == *k))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(HostError::MissingEnv {
                name: name.to_string(),
                missing: missing.join(", "),
            });
        }
        // Resuming brings back a known agent, so it skips the burst limit (a host
        // restart resumes many at once) but never a lockdown. The name is taken
        // while it waits, so release it first and put it back if the start fails.
        if let Err(locked) = self.refuse_if_locked() {
            // Under the forget policy a locked host does not keep it for later.
            lock(&self.awaiting).remove(name);
            self.save();
            return Err(locked);
        }
        lock(&self.awaiting).remove(name);
        let mut spec = spec_of(rec.clone(), env);
        if spec.transport == Transport::Acp {
            spec.mcp = mcp;
        }
        let started = self.start(spec).and_then(|info| {
            self.undo_if_locked(&info.name)?;
            Ok(info)
        });
        match started {
            Ok(info) => Ok(info),
            Err(e @ HostError::LockedDown(_)) => Err(e),
            Err(e) => {
                lock(&self.awaiting).insert(rec.name.clone(), rec);
                Err(e)
            }
        }
    }

    /// Keeps the state file as it is, then stops every agent. Call when the
    /// host is going away: the agents are stopped but still listed, so the next
    /// start brings them back.
    pub fn shutdown(&self) {
        self.save();
        if let Some(state) = &self.state {
            state.freeze();
        }
        let panes: Vec<Arc<Pane>> = lock(&self.panes).values().cloned().collect();
        for pane in panes {
            pane.kill();
        }
        for pane in lock(&self.acp).values() {
            pane.kill();
        }
    }
}
