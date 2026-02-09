use clap::Parser;
use crossterm::event::{Event, KeyCode};
use env_logger::{Builder, Target};
use host::{
    adc::{Adc, Pressure, PressureUnit},
    controller::Controller,
    tmp::Tmp,
};
use log::*;
use plotters::prelude::*;
use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Styled},
    widgets::{Axis, Block, Chart, Clear, Dataset, GraphType, Padding, Paragraph, Widget},
};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Mutex;

type AnyError = Box<dyn std::error::Error>;

/// The maximum backing pressure supported by the TMP in mbar.
///
/// At pressures above this, the TMP can't safely run.
///
/// The manual for the Preiffer TMH 071 P states a maximum backing pressure of
/// 18 mbar, but I'll use an even lower pressure here just to be safe.
const TMP_MAXIMUM_BACKING_PRESSURE_MBAR: f64 = 12.0;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AnyError> {
    init_logging()?;

    let args = Arguments::parse();
    let mut terminal = ratatui::init();

    // Use an `async` block to ensure we call `ratatui::restore` on error.
    let result: Result<(), AnyError> = async {
        let controller = Arc::new(
            Controller::new(&args.device_path)
                .inspect_err(|_| error!("failed to create controller"))?,
        );
        let app = Arc::new(App {
            adc: Adc::new(Arc::clone(&controller), args.adc_gauge_number),
            start_time: Instant::now(),
            state: Mutex::new(State::default()),
            tmp: Tmp::new(&args.tmp_address, Arc::clone(&controller)),
        });

        // Ensure the ADC is using millibar pressure units.
        app.adc.set_pressure_unit(PressureUnit::Millibar).await?;

        // Spawn the update task.
        //
        // Responsible for updating `app.state` and writing measurements to disk.
        {
            let app = Arc::clone(&app);
            tokio::spawn(async move {
                loop {
                    if let Err(e) = app.update_state().await {
                        panic!("failed to update application state: {}", e);
                    }

                    // Failure to graph or log pressure data shouldn't cause
                    // the application to panic. Just log an error message.
                    let _ = app
                        .graph_pressure()
                        .await
                        .inspect_err(|e| error!("failed to graph pressure: {}", e));
                    let _ = app
                        .log_pressure()
                        .await
                        .inspect_err(|e| error!("failed to log pressure: {}", e));

                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }

        // Spawn the UI task.
        //
        // Responsible for rendering the UI and handling user input.
        //
        // We `await` the completion of this task because it handles user input and
        // will return when the user has requested to quit the application.
        {
            let app = Arc::clone(&app);
            let _ = tokio::spawn(async move {
                loop {
                    match app.update_ui(&mut terminal).await {
                        Ok(exit) => {
                            if exit {
                                break;
                            }
                        }
                        Err(e) => panic!("failed to update ui: {}", e),
                    }

                    // Explicitly yield so the update task can run.
                    tokio::task::yield_now().await;
                }
            })
            .await?;
        }

        Ok(())
    }
    .await;

    ratatui::restore();
    result
}

/// Initialise `env_logger` to log to `out/app.log`.
fn init_logging() -> Result<(), std::io::Error> {
    let file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open("out/app.log")?;

    Builder::from_default_env()
        .target(Target::Pipe(Box::new(file)))
        .init();

    info!("Application started");
    Ok(())
}

#[derive(Parser)]
struct Arguments {
    /// The path to the controller device, e.g. /dev/tty.usbmodem11201.
    device_path: String,

    /// The gauge number to use when communicating with the ADC, e.g. 1.
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    adc_gauge_number: u8,

    /// The address of the TMP in the RS-485 Pfeiffer Vacuum Protocol, e.g. 001.
    ///
    /// This corresponds to TMP parameter 797.
    tmp_address: String,
}

#[derive(Debug)]
struct App {
    /// An interface for interacting with the Edwards ADC MkII.
    adc: Adc,

    /// The time at which the application started.
    ///
    /// Used to generate timestamps of pressure measurements.
    start_time: Instant,

    /// The application's state fetched from the vacuum devices.
    state: Mutex<State>,

    /// An interface for interacting with the Pfeiffer TMP.
    tmp: Tmp,
}

impl App {
    /// Graphs the pressure measurements over time.
    ///
    /// Saves the graph to `out/graph.png`.
    async fn graph_pressure(&self) -> Result<(), AnyError> {
        let state = self.state.lock().await;

        if state.pressures.len() == 0 {
            return Ok(());
        }

        // Determine the minimum and maximum pressures to set the y axis range.
        let mut min = f64::MAX;
        let mut max = f64::MIN;

        for (_, p) in &state.pressures {
            if p.value < min {
                min = p.value;
            }

            if p.value > max {
                max = p.value;
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
                0.0..state.pressures.last().copied().unwrap_or_default().0 / 60.0,
                min..max,
            )?;

        chart
            .configure_mesh()
            .axis_desc_style(("sans-serif", 18))
            .disable_x_mesh()
            .max_light_lines(5)
            .x_desc("Time (min)")
            .x_label_formatter(&|x| format!("{:.0}", x.floor()))
            .x_label_style(("sans-serif", 16))
            .y_desc("Pressure (mbar)")
            .y_label_style(("sans-serif", 16))
            .draw()?;

        chart.draw_series(LineSeries::new(
            // Convert seconds to minutes.
            state.pressures.iter().map(|(t, p)| (t / 60.0, p.value)),
            &BLUE,
        ))?;
        root.present()?;
        fs::rename("out/graph_new.png", "out/graph.png")?;

        // Update the file's modified time so VS Code refreshes it.
        fs::File::open("out/graph.png")?.set_modified(SystemTime::now())?;

        Ok(())
    }

    /// Logs the pressure measurements over time.
    ///
    /// Saves the log to `out/log.csv`.
    async fn log_pressure(&self) -> Result<(), std::io::Error> {
        let state = self.state.lock().await;
        fs::write(
            "out/log_new.csv",
            String::from("Time (s),Pressure (mbar)\n")
                + &state
                    .pressures
                    .iter()
                    .map(|(t, p)| format!("{},{}", t, p.value))
                    .collect::<Vec<String>>()
                    .join("\n"),
        )?;
        fs::rename("out/log_new.csv", "out/log.csv")?;
        Ok(())
    }

    /// Update `state` by fetching new data from the vacuum devices.
    async fn update_state(&self) -> Result<(), AnyError> {
        // Fetch all the data.
        let pressure = self.adc.get_pressure().await?;
        let tmp_current = self.tmp.get_current().await?;
        let tmp_current_rotation_speed = self.tmp.get_current_rotation_speed().await?;
        let tmp_running = self.tmp.is_running().await?;
        let tmp_target_rotation_speed = self.tmp.get_target_rotation_speed().await?;

        // Update the state.
        let mut state = self.state.lock().await;
        let mut pressures = std::mem::take(&mut state.pressures);
        pressures.push((self.start_time.elapsed().as_secs_f64(), pressure));
        *state = State {
            pressures,
            show_roughing_pump_confirmation: state.show_roughing_pump_confirmation,
            show_tmp_pressure_error: state.show_tmp_pressure_error,
            tmp_current,
            tmp_current_rotation_speed,
            tmp_running,
            tmp_target_rotation_speed,
        };

        Ok(())
    }

    /// Render the UI and handle user input.
    async fn update_ui(&self, terminal: &mut DefaultTerminal) -> Result<bool, AnyError> {
        let has_event = crossterm::event::poll(Duration::from_secs(0))?;
        let mut state = self.state.lock().await;

        if has_event {
            let event = crossterm::event::read()?;
            if let Event::Key(event) = event
                && event.is_press()
            {
                match event.code {
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        if state.show_roughing_pump_confirmation {
                            // If the "confirm the roughing pump is running"
                            // message is visible then we know the chamber
                            // pressure is low enough. The user has pressed 'P'
                            // again to confirm, so we can start the TMP.
                            self.tmp.set_running(true).await?;
                            state.show_roughing_pump_confirmation = false;
                            state.tmp_running = true;
                        }
                    }

                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                        if state.show_roughing_pump_confirmation {
                            state.show_roughing_pump_confirmation = false;
                        } else if state.show_tmp_pressure_error {
                            state.show_tmp_pressure_error = false;
                        } else {
                            return Ok(true);
                        }
                    }

                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        if state.tmp_running {
                            // If the TMP is already running, turn it off.
                            self.tmp.set_running(false).await?;
                        } else if let Some((_, pressure)) = state.pressures.last() {
                            if pressure.value <= TMP_MAXIMUM_BACKING_PRESSURE_MBAR {
                                // The chamber pressure being below the TMP's
                                // maximum backing pressure doesn't guarantee
                                // the roughing pump is running (e.g. it may
                                // have been running earlier without the chamber
                                // being brought back to atmospheric pressure).
                                // We need to confirm with the user that it's
                                // running, otherwise pressure will build up in
                                // the TMP's outlet and potentially damage it.
                                state.show_roughing_pump_confirmation = true;
                            } else {
                                // The chamber pressure isn't low enough.
                                state.show_tmp_pressure_error = true;
                            }
                        }
                    }

                    _ => {}
                }
            }
        }

        terminal.draw(|frame| frame.render_widget(&(*state), frame.area()))?;
        Ok(false)
    }
}

#[derive(Debug, Default)]
struct State {
    /// Measurements of the pressure in the chamber over time.
    ///
    /// The first entry is the time the measurement was taken in seconds since
    /// the start of the application and the second is the pressure.
    pressures: Vec<(f64, Pressure)>,

    /// If the "confirm the roughing pump is running" message should be shown.
    show_roughing_pump_confirmation: bool,

    /// If the "chamber pressure too high for TMP" error should be shown.
    show_tmp_pressure_error: bool,

    /// The current draw of the TMP in amperes.
    tmp_current: f32,

    /// The current rotation speed of the TMP in hertz.
    tmp_current_rotation_speed: u16,

    /// If the TMP is running.
    tmp_running: bool,

    /// The target totation speed of the TMP in hertz.
    tmp_target_rotation_speed: u16,
}

impl Widget for &State {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Color definitions.
        const BLOCK_TITLE_STYLE: Style = Style::new().add_modifier(Modifier::BOLD).fg(Color::White);
        const CONFIRMATION_STYLE: Style = Style::new().fg(Color::Yellow);
        const ERROR_STYLE: Style = Style::new().fg(Color::Red);
        const SUBTLE_TEXT_STYLE: Style = Style::new().fg(Color::Rgb(96, 96, 96));
        const VARIABLE_STYLE: Style = Style::new().fg(Color::Blue);

        // Main layout.
        let [main, shortcuts_bar] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
        let [left, right] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(36)]).areas(main);

        // Pressure graph.
        let pressure_string = self
            .pressures
            .last()
            .map_or(String::from("?"), |(_, p)| format!("{:.1e}", p.value));
        let pressure_block = Block::bordered()
            .padding(Padding::symmetric(2, 1))
            .title(vec![
                "Pressure: ".set_style(BLOCK_TITLE_STYLE),
                format!("{} mbar", pressure_string).set_style(VARIABLE_STYLE),
            ]);

        if self.pressures.len() < 2 {
            pressure_block.render(left, buf);

            let text = "Measuring pressure...";
            Paragraph::new(text.set_style(SUBTLE_TEXT_STYLE)).render(
                left.centered(
                    Constraint::Length(text.len().try_into().unwrap()),
                    Constraint::Length(1),
                ),
                buf,
            );
        } else {
            // Show roughly the past minute of measurements.
            //
            // Measurements seem to be taken every ~2 seconds, thus 30.
            let start = self.pressures.len().saturating_sub(30);
            let data: Vec<(f64, f64)> = self.pressures[start..]
                .iter()
                .map(|&(t, p)| (t, p.value))
                .collect();

            // The timestamp of the first and last measurements.
            let first = data.first().unwrap().0;
            let last = data.last().unwrap().0;

            // The values of the minimum and maximum measurements.
            let mut min = data
                .iter()
                .min_by(|(_, a), (_, b)| a.total_cmp(b))
                .unwrap()
                .1;
            let mut max = data
                .iter()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))
                .unwrap()
                .1;

            // Sometimes all the measurements are equal, e.g. if few have been
            // taken and the chamber is at a stable pressure. In that case, add
            // some padding to the min/max values so the y axis has a non-zero
            // height. We use 10% to ensure at least one digit is different in
            // the y axis labels when they're rendered in scientific notation.
            if min == max {
                min *= 0.9;
                max *= 1.1;
            }

            Chart::new(vec![
                Dataset::default()
                    .data(&data)
                    .graph_type(GraphType::Line)
                    .style(VARIABLE_STYLE),
            ])
            .x_axis(
                Axis::default()
                    .bounds([first, last])
                    .labels(
                        [
                            format!("{:.0}", first),
                            format!("{:.0}", (first + last) / 2.0),
                            format!("{:.0}", last),
                        ]
                        .map(|l| l.set_style(SUBTLE_TEXT_STYLE)),
                    )
                    .title("Time (s)"),
            )
            .y_axis(
                Axis::default()
                    .bounds([min, max])
                    .labels(
                        [
                            format!("{:.1e}", min),
                            format!("{:.1e}", (min + max) / 2.0),
                            format!("{:.1e}", max),
                        ]
                        .map(|l| l.set_style(SUBTLE_TEXT_STYLE)),
                    )
                    .title("Pressure (mbar)"),
            )
            .block(pressure_block)
            .render(left, buf);
        }

        // TMP block.
        let tmp_layout = Layout::vertical([Constraint::Length(7)])
            .flex(Flex::Start)
            .split(right);
        Paragraph::new(vec![
            vec![
                "Current: ".into(),
                format!("{:.2} A", self.tmp_current).set_style(VARIABLE_STYLE),
            ]
            .into(),
            vec![
                "Rotation speed: ".into(),
                format!(
                    "{} / {} Hz",
                    self.tmp_current_rotation_speed, self.tmp_target_rotation_speed
                )
                .set_style(VARIABLE_STYLE),
            ]
            .into(),
            vec![
                "Running: ".into(),
                (if self.tmp_running { "Yes" } else { "No" }).set_style(VARIABLE_STYLE),
            ]
            .into(),
        ])
        .block(
            Block::bordered()
                .padding(Padding::symmetric(2, 1))
                .title("TMP".set_style(BLOCK_TITLE_STYLE)),
        )
        .render(tmp_layout[0], buf);

        // Shortcuts bar.
        Paragraph::new(
            format!(
                "[Esc / Q]: {}   [P]: Turn TMP {}",
                if self.show_roughing_pump_confirmation || self.show_tmp_pressure_error {
                    "Dismiss"
                } else {
                    "Quit"
                },
                if self.tmp_running { "off" } else { "on" }
            )
            .set_style(SUBTLE_TEXT_STYLE),
        )
        .centered()
        .render(shortcuts_bar, buf);

        // Roughing pump confirmation.
        if self.show_roughing_pump_confirmation {
            let lines = [
                "Confirm that the roughing pump is running.",
                "",
                "Press [C] to confirm or [Esc / Q] to cancel.",
            ];
            let width: u16 = lines
                .iter()
                .map(|l| l.len())
                .max()
                .unwrap()
                .try_into()
                .unwrap();
            let height: u16 = lines.len().try_into().unwrap();
            let area = area.centered(
                Constraint::Length(width + 6),
                Constraint::Length(height + 4),
            );
            Clear.render(area, buf);
            Paragraph::new(lines.join("\n"))
                .block(
                    Block::bordered()
                        .padding(Padding::symmetric(2, 1))
                        .title("Confirm".set_style(BLOCK_TITLE_STYLE)),
                )
                .centered()
                .style(CONFIRMATION_STYLE)
                .render(area, buf);
        }

        // TMP pressure error.
        if self.show_tmp_pressure_error {
            let text = format!(
                "The chamber pressure must be below {:.0} mbar to run the TMP.",
                TMP_MAXIMUM_BACKING_PRESSURE_MBAR
            );
            let text_length: u16 = text.len().try_into().unwrap();
            let area = area.centered(
                Constraint::Length(text_length + 6),
                Constraint::Length(1 + 4),
            );
            Clear.render(area, buf);
            Paragraph::new(text)
                .block(
                    Block::bordered()
                        .padding(Padding::symmetric(2, 1))
                        .title("Error".set_style(BLOCK_TITLE_STYLE)),
                )
                .centered()
                .set_style(ERROR_STYLE)
                .render(area, buf);
        }
    }
}
