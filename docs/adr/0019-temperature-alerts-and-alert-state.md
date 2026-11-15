# ADR-0019: Temperature alerts and durable alert state

Status: accepted (alert state contents amended by ADR-0021)

## Context
Issue #5 needs a notification when a room stays too hot or too cold, without
repeats at the limit and without a duplicate first notification after a
restart. ADR-0011 calls for a JSON state file; ADR-0017 kept alert state in
memory for now.

## Decision
- Each `[[thermometers]]` entry may set `min_temperature` and
  `max_temperature` (°C), both or neither, finite, min below max; a room
  without them has no temperature rule. Initial ranges: bedroom 18–20,
  office and living room 19–22.
- Only temperature readings of configured thermometers feed the rule, ordered
  by their own timestamps; a reading not newer than the last is ignored.
- Onset: every reading outside [min, max] for 15 min. Resolved: every reading
  inside [min, max] for 15 min. A hold completes only when a reading proves
  it, never by elapsed time alone. Time hysteresis was chosen over a
  temperature dead band, which would keep reporting a room that settled just
  inside its range as still too hot.
- When the room crosses to the opposite extreme and holds there 15 min, the
  alert switches direction with a new first notification.
- A reminder follows 12 h after the last delivered notification and names the
  latest temperature. Without readings an alert neither resolves nor stops
  reminding; the offline rule reports the silence.
- Notifications name the room, the direction, the temperature and the range,
  and follow ADR-0017's rules: decided just before sending, recorded when
  ntfy accepts them, retried at the next check.
- `alerter` persists the delivered state of both rules (offline: last seen
  and notified at; temperature: direction, temperature and notified at) to
  `[alerter] state_file`, default `/data/alerts.json`, on the `alerter-data`
  volume. Format `{"version":1,...}`, written to a temp file, fsynced, renamed
  and the directory fsynced. This replaces ADR-0017's in-memory state; sensors
  with no persisted offline alert still count as seen at startup.
- The last notified temperature is persisted so reminders keep their schedule
  after a restart even without new readings.
- The state is saved after each accepted notification. A failed save is
  logged, kept in memory and retried at every check.
- A missing file is an empty state. An unreadable, corrupt or unknown-version
  file is logged and ignored, and the next save replaces it: losing it costs
  duplicates, while refusing to start would silence every alert.
- The file sits behind an `AlertStateStore` port in `use-cases`, implemented
  by a new `state-file` crate; `alerter` does not depend on `storage`.

## Consequences
- A restart during an active alert sends no duplicate first notification and
  keeps the reminder schedule; resolved and back messages still follow.
- Pending holds are in memory: a restart during one delays onset or
  resolution by up to 15 min.
- A crash between delivery and save, or a lost state file, can repeat a
  first notification.
- Removing a sensor or its range drops its alert without a resolved message.
- Older services reject the new thermometer keys, so rollout is staged:
  update every image first, then add the ranges and restart.
