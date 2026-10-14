use super::codec::{decode_job, decode_job_unchecked};

/// What to do with one datagram from LightBurn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Send the original datagram to the controller.
    Forward,
    /// Do not send it. `reply` is a synthetic response so LightBurn does not wait out the timeout.
    Drop { reply: &'static [u8] },
}

const JOB_NAK: &[u8] = &[0x46];
const PANEL_ACK: &[u8] = &[0xCC];

/// Real-time panel commands that energize the tube or start a stored job.
const PANEL_FIRE: &[&[u8]] = &[
    &[0xA5, 0x50, 0x05], // pulse down
    &[0xA5, 0x51, 0x05], // pulse up
    &[0xA5, 0x50, 0x06], // start / pause down
    &[0xA5, 0x51, 0x06], // start / pause up
    &[0xA5, 0x50, 0x12], // laser gate down
    &[0xA5, 0x51, 0x12], // laser gate up
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Swizzled RD stream, UDP 50200. Carries jobs and status polls.
    Job,
    /// Unscrambled keypad interface, UDP 50207. Jog, frame, pulse, start.
    Panel,
}

pub fn judge(channel: Channel, datagram: &[u8], locked: bool) -> Verdict {
    if !locked || datagram.is_empty() {
        return Verdict::Forward;
    }
    match channel {
        Channel::Panel => {
            if panel_fires(datagram) {
                Verdict::Drop { reply: PANEL_ACK }
            } else {
                Verdict::Forward
            }
        }
        Channel::Job => {
            if job_fires(datagram) {
                Verdict::Drop { reply: JOB_NAK }
            } else {
                Verdict::Forward
            }
        }
    }
}

fn panel_fires(datagram: &[u8]) -> bool {
    PANEL_FIRE.iter().any(|needle| contains(datagram, needle))
}

fn job_fires(datagram: &[u8]) -> bool {
    let clear = decode_job(datagram).or_else(|| decode_job_unchecked(datagram));
    let Some(clear) = clear else {
        // Too short to be a command. Let the controller ignore it.
        return false;
    };
    contains_job_fire(&clear)
}

pub fn contains_job_fire(clear: &[u8]) -> bool {
    // D8 00 starts a process file. D8 03 resumes one. Both end with the tube enabled.
    if clear.windows(2).any(|pair| pair == [0xD8, 0x00] || pair == [0xD8, 0x03]) {
        return true;
    }
    // Path commands that move with the beam on. Travel moves are 0x88–0x8B.
    clear
        .iter()
        .any(|b| matches!(b, 0xA8 | 0xA9 | 0xAA | 0xAB))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// In-band "stop process" and the panel Stop key, sent when a session ends.
pub(crate) fn job_stop_clear() -> &'static [u8] {
    &[0xD8, 0x01]
}

pub fn panel_stop() -> &'static [&'static [u8]] {
    &[&[0xA5, 0x50, 0x09], &[0xA5, 0x51, 0x09]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::codec::encode_job;

    #[test]
    fn locked_forwards_travel_and_frame() {
        let travel = encode_job(&[
            0x88, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00,
        ]);
        assert_eq!(judge(Channel::Job, &travel, true), Verdict::Forward);
        assert_eq!(
            judge(Channel::Panel, &[0xA5, 0x53, 0x00], true),
            Verdict::Forward
        );
        assert_eq!(
            judge(Channel::Panel, &[0xA5, 0x50, 0x01], true),
            Verdict::Forward
        );
        assert_eq!(judge(Channel::Panel, &[0xCC], true), Verdict::Forward);
        let poll = encode_job(&[0xDA, 0x00, 0x04, 0x21]);
        assert_eq!(judge(Channel::Job, &poll, true), Verdict::Forward);
    }

    #[test]
    fn locked_drops_cuts_pulse_and_start() {
        let cut = encode_job(&[
            0xA8, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00,
        ]);
        assert!(matches!(
            judge(Channel::Job, &cut, true),
            Verdict::Drop { .. }
        ));
        let start = encode_job(&[0xD8, 0x00]);
        assert!(matches!(
            judge(Channel::Job, &start, true),
            Verdict::Drop { .. }
        ));
        assert!(matches!(
            judge(Channel::Panel, &[0xA5, 0x50, 0x05], true),
            Verdict::Drop { .. }
        ));
        assert!(matches!(
            judge(Channel::Panel, &[0xA5, 0x50, 0x06], true),
            Verdict::Drop { .. }
        ));
        assert!(matches!(
            judge(Channel::Panel, &[0xA5, 0x50, 0x12], true),
            Verdict::Drop { .. }
        ));
    }

    #[test]
    fn unlocked_forwards_cuts() {
        let cut = encode_job(&[0xA8, 0x01, 0x02]);
        assert_eq!(judge(Channel::Job, &cut, false), Verdict::Forward);
        assert_eq!(
            judge(Channel::Panel, &[0xA5, 0x50, 0x05], false),
            Verdict::Forward
        );
    }
}
