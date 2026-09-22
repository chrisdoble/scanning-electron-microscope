/// A test program for the host::oscilloscope::Oscilloscope type.
use host::oscilloscope::Oscilloscope;

type AnyError = Box<dyn std::error::Error>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    let oscilloscope = Oscilloscope::new().await?;
    oscilloscope.reset().await?;
    oscilloscope.set_vertical_scale(1.).await?;
    println!("{}", oscilloscope.get_voltage().await?);

    Ok(())
}
