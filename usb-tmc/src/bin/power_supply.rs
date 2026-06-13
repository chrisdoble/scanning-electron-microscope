use usb_tmc::UsbTmcDevice;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Rigol DP900 series
    let power_supply = UsbTmcDevice::open(0x1ab1, 0xa4a8, None).await?;

    // Set channel 1's voltage to 0.1 V
    power_supply.write_str(":APPLy CH1,0.1").await?;

    // Turn channel 1 on
    power_supply.write_str(":OUTPut CH1,ON").await?;

    // Measure the voltage, current, and power at the output terminal of channel 1
    println!("{}", power_supply.query_str(":MEASure:ALL? CH1").await?);

    Ok(())
}
