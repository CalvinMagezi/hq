# Copilot credits per step

The web chat shows an approximate Copilot credit figure for each agent step and a total in the reply footer.

## How it is measured

GitHub exposes one account-wide counter (`credits_used`, read by `hq_llm::copilot_usage::fetch_quota`). The session loop reads it before the first step of a run and after every step, and emits `SessionEvent::StepCredits` right after `TurnEnd`. The delta is the later reading minus the earlier one, floored at 0. Each step's closing reading is the next step's baseline, so it costs one HTTP call per step.

The reads run beside the work (the closing read overlaps tool execution), each has a 3 second timeout, and a failed or timed out read gives `delta: null`, shown as `n/a`. A read never fails or holds up a step, except that the last step of a reply waits for its own closing read (at most 3 seconds) before the run returns.

Only steps served by the in-process `copilot` backend emit the event. Other backends, and the Copilot CLI backend, emit nothing.

## Why it is approximate

- The counter is shared by everything on the seat, so other use during a step is counted.
- GitHub updates it with a delay, so a step's credits can land in a neighbouring step or read as `<1`.
- Tiny steps round to `<1 credit`.

The UI says so in the badge tooltip.

## Where it shows and is stored

- Live: the `step_credits` WebSocket event, attached to the first tool call of the step.
- Saved: `meta.step_credits` on the reply message, so a reload shows the same badges.
- Telegram, Discord and the CLI ignore the event.

## Config

```yaml
copilot_usage:
  per_step: true   # default; false turns the per-step reads off
```

Only sessions built from `chat_session_config` (the web chat) read the counter.
