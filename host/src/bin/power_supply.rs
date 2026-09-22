/// A test program for the host::power_supply::PowerSupply type.
use clap::Parser;
use host::{controller::Controller, power_supply::PowerSupply};
use std::sync::Arc;

type AnyError = Box<dyn std::error::Error>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    let args = Arguments::parse();
    let controller = Arc::new(Controller::new(&args.device_path)?);
    let power_supply = PowerSupply::new(controller).await?;

    Ok(())
}

#[derive(Parser)]
struct Arguments {
    /// The path to the controller device, e.g. /dev/tty.usbmodem11201.
    device_path: String,
}
