use host::{
    adc::{Adc, PressureUnit},
    controller::Controller,
};
use log::*;
use plotters::prelude::*;
use std::{
    f64, fs,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Connect to the ADC and set the pressure units to mbar.
    let controller = Arc::new(
        Controller::new("/dev/tty.usbmodem11201")
            .inspect_err(|_| error!("failed to create controller"))?,
    );
    let adc = Adc::new(Arc::clone(&controller));
    adc.set_pressure_unit(PressureUnit::Millibar).await?;

    // Pressure measurements over time.
    //
    // The first entry is the time the measurement was taken in seconds since
    // the start of the program and the second is the pressure in mbar.
    let mut measurements: Vec<(f64, f64)> = vec![];

    // Record measurements until the program is terminated.
    let start_time = Instant::now();
    loop {
        measurements.push((
            start_time.elapsed().as_secs_f64(),
            adc.get_pressure(1).await?.value,
        ));
        graph_measurements(&measurements)?;
        log_measurements(&measurements)?;
        thread::sleep(Duration::from_secs(1));
    }
}

fn graph_measurements(measurements: &Vec<(f64, f64)>) -> Result<(), Box<dyn std::error::Error>> {
    if measurements.len() == 0 {
        return Ok(());
    }

    // Determine the minimum and maximum pressures to set the y axis range.
    let mut min = f64::MAX;
    let mut max = f64::MIN;

    for &(_, p) in measurements {
        if p < min {
            min = p;
        }

        if p > max {
            max = p;
        }
    }

    // Graph the measurements.
    //
    // If we overwrite the existing graph file (`out/graph.png`), killing the
    // program can sometimes leave the file in an invalid state. To avoid this,
    // temporarily write to another file and rename it once we're done.
    let root = BitMapBackend::new("out/graph_new.png", (1024, 768)).into_drawing_area();
    root.fill(&WHITE)?;

    let mut chart = ChartBuilder::on(&root)
        .caption("Chamber pressure", ("sans-serif", 24))
        .margin(10)
        .set_label_area_size(LabelAreaPosition::Left, 60)
        .set_label_area_size(LabelAreaPosition::Bottom, 60)
        .build_cartesian_2d(
            // Convert seconds to minutes.
            0.0..measurements.last().unwrap().0 / 60.0,
            min..max,
        )?;

    chart
        .configure_mesh()
        .axis_desc_style(("sans-serif", 18))
        .disable_x_mesh()
        .max_light_lines(5)
        .x_desc("Time (min)")
        .x_label_formatter(&|x| format!("{:.0}", x))
        .x_label_style(("sans-serif", 16))
        .y_desc("Pressure (mbar)")
        .y_label_style(("sans-serif", 16))
        .draw()?;

    chart.draw_series(LineSeries::new(
        // Convert seconds to minutes.
        measurements.iter().map(|&(t, p)| (t / 60.0, p)),
        &BLUE,
    ))?;
    root.present()?;
    fs::rename("out/graph_new.png", "out/graph.png")?;

    // Update the file's modified time so VS Code refreshes it.
    fs::File::open("out/graph.png")?.set_modified(SystemTime::now())?;

    Ok(())
}

fn log_measurements(measurements: &Vec<(f64, f64)>) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(
        "out/log_new.csv",
        String::from("Time (s),Pressure (mbar)\n")
            + &measurements
                .iter()
                .map(|(t, p)| format!("{},{}", t, p))
                .collect::<Vec<String>>()
                .join("\n"),
    )?;
    fs::rename("out/log_new.csv", "out/log.csv")?;
    Ok(())
}
