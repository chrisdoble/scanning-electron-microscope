use controller::{Controller, Destination};
use log::*;
use std::sync::Arc;

mod controller;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let controller = Arc::new(
        Controller::new("/dev/tty.usbmodem11201")
            .inspect_err(|_| error!("failed to create controller"))?,
    );

    controller.send_command(Destination::ADC, "?GA1").await?;
    controller
        .send_command(Destination::TMP, "0010031102=?100")
        .await?;

    Ok(())
}
