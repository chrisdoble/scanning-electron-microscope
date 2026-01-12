use adc::{Adc, PressureUnit};
use controller::Controller;
use log::*;
use std::sync::Arc;

mod adc;
mod controller;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let controller = Arc::new(
        Controller::new("/dev/tty.usbmodem11201")
            .inspect_err(|_| error!("failed to create controller"))?,
    );

    let adc = Adc::new(Arc::clone(&controller));
    adc.set_pressure_unit(PressureUnit::Millibar).await?;
    let pressure = adc.get_pressure(1).await?;
    println!("{:?}", pressure);

    Ok(())
}
