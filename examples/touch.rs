#![no_std]
#![no_main]

use cst816s::{TouchEvent, TouchGesture, CST816S};
use panic_probe as _;
use rp235x_hal::{self as hal, entry, gpio, spi, Clock, I2C};

use display_interface_spi::SPIInterface;
use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::prelude::{Drawable, Primitive};
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle};
use embedded_graphics_core::draw_target::DrawTarget;
use embedded_graphics_core::pixelcolor::Rgb565;
use embedded_graphics_core::prelude::RgbColor;
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal_bus::spi::ExclusiveDevice;
use mipidsi::models::ST7789;
use mipidsi::options::ColorInversion::Inverted;
use mipidsi::options::{Orientation, Rotation};
use mipidsi::Builder;
use rp235x_hal::block::ImageDef;
use rp235x_hal::fugit::RateExtU32;
use rp235x_hal::gpio::{FunctionI2C, FunctionSpi, Pin};

/// Tell the Boot ROM about our application
#[link_section = ".start_block"]
#[used]
pub static IMAGE_DEF: ImageDef = hal::block::ImageDef::secure_exe();

/// External high-speed crystal on the Raspberry Pi Pico 2 board is 12 MHz.
/// Adjust if your board has a different frequency
const XTAL_FREQ_HZ: u32 = 12_000_000u32;

const SCREEN_WIDTH: u32 = 240;
const SCREEN_HEIGHT: u32 = 320;
const HALF_SCREEN_WIDTH: u32 = SCREEN_WIDTH / 2;
const MIN_SCREEN_DIM: u32 = SCREEN_HEIGHT;
const SCREEN_RADIUS: u32 = (MIN_SCREEN_DIM / 2) as u32;

#[entry]
fn main() -> ! {
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let _core = cortex_m::Peripherals::take().unwrap();

    // Set up the watchdog driver - needed by the clock setup code
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);

    // Configure the clocks
    //
    // The default is to generate a 125 MHz system clock
    let clocks = hal::clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    // The single-cycle I/O block controls our GPIO pins
    let sio = hal::Sio::new(pac.SIO);

    // Set the pins up according to their function on this particular board
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let dc = pins.gpio16.into_push_pull_output();
    let cs = pins.gpio17.into_push_pull_output();
    let sck = pins.gpio18.into_function::<FunctionSpi>();
    let mosi = pins.gpio19.into_function::<FunctionSpi>();
    let rst = pins
        .gpio20
        .into_push_pull_output_in_state(gpio::PinState::High);
    let miso = pins.gpio4.into_function::<FunctionSpi>();
    let mut bl = pins.gpio15.into_push_pull_output();

    let sda_pin: Pin<_, FunctionI2C, _> = pins.gpio12.reconfigure();
    let scl_pin: Pin<_, FunctionI2C, _> = pins.gpio13.reconfigure();
    let tint = pins.gpio29.into_pull_down_input();
    let trst = pins
        .gpio21
        .into_push_pull_output_in_state(gpio::PinState::High);

    let i2c = I2C::i2c0(
        pac.I2C0,
        sda_pin,
        scl_pin,
        400.kHz(),
        &mut pac.RESETS,
        &clocks.system_clock,
    );

    let spi = spi::Spi::<_, _, _, 8>::new(pac.SPI0, (mosi, miso, sck)).init(
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
        16_000_000u32.Hz(),
        embedded_hal::spi::MODE_0,
    );

    let mut delay = hal::Timer::new_timer0(pac.TIMER0, &mut pac.RESETS, &clocks);

    bl.set_high().ok();

    let spi_device = ExclusiveDevice::new_no_delay(spi, cs).unwrap();
    let di = SPIInterface::new(spi_device, dc);

    let mut display = Builder::new(ST7789, di)
        .reset_pin(rst)
        .orientation(Orientation::new().rotate(Rotation::Deg90))
        .invert_colors(Inverted)
        .display_size(SCREEN_WIDTH as u16, SCREEN_HEIGHT as u16)
        .init(&mut delay)
        .unwrap();

    // Give display time to fully initialize
    delay.delay_ns(100_000_000u32);

    // Clear display to black
    display.clear(Rgb565::BLACK).unwrap();

    let mut touchpad = CST816S::new(i2c, tint, trst);
    touchpad.setup(&mut delay).unwrap();

    let mut refresh_count = 0;

    loop {
        if let Some(evt) = touchpad.read_one_touch_event(true) {
            refresh_count += 1;
            //hprintln!("{:?}", evt).unwrap();

            draw_marker(&mut display, &evt, Rgb565::MAGENTA);
            let _vibe_time = match evt.gesture {
                TouchGesture::LongPress => {
                    refresh_count = 1000;
                    50_000
                }
                TouchGesture::SingleClick => 5_000,
                _ => 0,
            };

            if refresh_count > 40 {
                draw_background(&mut display);
                refresh_count = 0;
            }
        } else {
            delay.delay_us(1u32);
        }
    }
}

fn draw_background(display: &mut impl DrawTarget<Color = Rgb565>) {
    let clear_bg = Rectangle::new(Point::new(0, 0), Size::new(SCREEN_WIDTH, SCREEN_HEIGHT))
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK));
    clear_bg.draw(display).map_err(|_| ()).unwrap();

    let center_circle = Circle::new(
        Point::new(HALF_SCREEN_WIDTH as i32, (SCREEN_HEIGHT / 2) as i32),
        SCREEN_RADIUS,
    )
    .into_styled(PrimitiveStyle::with_stroke(Rgb565::YELLOW, 4));
    center_circle.draw(display).map_err(|_| ()).unwrap();
}

const SWIPE_LENGTH: i32 = 20;
const SWIPE_WIDTH: i32 = 2;

/// Draw an indicator of the kind of gesture we detected
fn draw_marker(display: &mut impl DrawTarget<Color = Rgb565>, event: &TouchEvent, color: Rgb565) {
    let x_pos = event.x;
    let y_pos = event.y;

    match event.gesture {
        TouchGesture::SlideLeft | TouchGesture::SlideRight => {
            Rectangle::new(
                Point::new(x_pos - SWIPE_LENGTH, y_pos - SWIPE_WIDTH),
                Size::new((x_pos + SWIPE_LENGTH) as u32, (y_pos + SWIPE_WIDTH) as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .map_err(|_| ())
            .unwrap();
        }
        TouchGesture::SlideUp | TouchGesture::SlideDown => {
            Rectangle::new(
                Point::new(x_pos - SWIPE_WIDTH, y_pos - SWIPE_LENGTH),
                Size::new((x_pos + SWIPE_WIDTH) as u32, (y_pos + SWIPE_LENGTH) as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .map_err(|_| ())
            .unwrap();
        }
        TouchGesture::SingleClick => Circle::new(Point::new(x_pos, y_pos), 20)
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .map_err(|_| ())
            .unwrap(),
        TouchGesture::LongPress => {
            Circle::new(Point::new(x_pos, y_pos), 40)
                .into_styled(PrimitiveStyle::with_stroke(color, 4))
                .draw(display)
                .map_err(|_| ())
                .unwrap();
        }
        _ => {}
    }
}
