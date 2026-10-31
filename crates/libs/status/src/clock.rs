use unbound_datetime::MinuteTimer;
use unbound_inbox::Outbox;

use crate::Update;

pub(crate) fn follow_minutes(outbox: &Outbox<Update>) -> rustix::io::Result<()> {
    let timer = MinuteTimer::new()?;
    loop {
        timer.wait()?;
        if !outbox.send(Update::Minute) {
            return Ok(());
        }
    }
}
