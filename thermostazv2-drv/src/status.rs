use crate::ThermostazvResult;
use thermostazv2_lib::Cmd;

pub type SWatchSender = tokio::sync::watch::Sender<Cmd>;
pub type SWatchReceiver = tokio::sync::watch::Receiver<Cmd>;
pub type SCmdSender = async_channel::Sender<Cmd>;
pub type SCmdReceiver = async_channel::Receiver<Cmd>;

pub async fn smanager(
    recv_cmd: SCmdReceiver,
    pub_state: SWatchSender,
    mut shutdown_receiver: tokio::sync::watch::Receiver<bool>,
) -> ThermostazvResult {
    loop {
        tokio::select! {
            _ = shutdown_receiver.changed() => return Ok(()),
            new = recv_cmd.recv() => if let Ok(new) = new {
                pub_state.send_if_modified(|old: &mut Cmd| {
                    if *old == new {
                        false
                    } else {
                        *old = new;
                        true
                    }
                });
            }
        }
    }
}
