use std::time::Duration;

use usb_tmc::UsbTmcDevice;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Rigol DHO800/900 series
    let scope = UsbTmcDevice::open(0x1ab1, 0x044d, None).await?;

    // Stop the scope
    scope.write_str(":STOP").await?;

    // Clear the screen
    scope.write_str(":CLEar").await?;

    // Show channel 1
    scope.write_str(":CHANnel1:DISPLAY ON").await?;

    // Set the offset of channel 1 to 0 V
    scope.write_str(":CHANnel1:OFFSet 0").await?;

    // Set the vertical scale of channel 1 to 1 V/div
    scope.write_str(":CHANnel1:SCALe 1").await?;

    // Clear all measurement items
    scope.write_str(":MEASure:CLEar").await?;

    // Run the scope
    scope.write_str(":RUN").await?;

    // Start calculating the average voltage of channel 1
    scope.write_str(":MEASure:ITEM? VAVG,CHANnel1").await?;

    // Give the scope some time to sample
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Query the average voltage of channel 1
    println!("{}", scope.query_str(":MEASure:ITEM? VAVG,CHANnel1").await?);

    Ok(())
}
