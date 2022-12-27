use thermostazv2_lib::Cmd;

pub type SWatchSender = tokio::sync::watch::Sender<Cmd>;
pub type SWatchReceiver = tokio::sync::watch::Receiver<Cmd>;
pub type SCmdSender = async_channel::Sender<Cmd>;
pub type SCmdReceiver = async_channel::Receiver<Cmd>;

pub async fn manager(recv_cmd: SCmdReceiver, pub_state: SWatchSender) {
    while let Ok(new) = recv_cmd.recv().await {
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
