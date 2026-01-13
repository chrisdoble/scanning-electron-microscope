use adc::{Adc, PressureUnit};
use controller::Controller;
use log::*;
use std::sync::Arc;
use tmp::Tmp;

mod adc;
mod controller;
mod tmp;

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
    println!("ADC pressure: {}", pressure);

    let tmp = Tmp::new("001", Arc::clone(&controller));
    println!(
        "TMP operating hours: {} h",
        tmp.get_operating_hours().await?
    );

    Ok(())
}
