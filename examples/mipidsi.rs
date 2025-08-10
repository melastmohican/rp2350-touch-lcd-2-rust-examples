#![no_std]
#![no_main]

use rp235x_hal::{self as hal, entry, gpio, spi, Clock};

use panic_probe as _;
use display_interface_spi::SPIInterface;
use embedded_graphics::draw_target::DrawTargetExt;
use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::image::{Image, ImageRaw, ImageRawLE};
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_8X13};
use embedded_graphics::mono_font::{MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::prelude::{Drawable, Primitive};
use embedded_graphics::primitives::{Circle, CornerRadii, Ellipse, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, RoundedRectangle, Triangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_graphics::transform::Transform;
use embedded_graphics_core::draw_target::DrawTarget;
use embedded_graphics_core::geometry::Dimensions;
use embedded_graphics_core::pixelcolor::{Rgb565, WebColors};
use embedded_graphics_core::prelude::RgbColor;
use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_text::alignment::{HorizontalAlignment, VerticalAlignment};
use embedded_text::style::TextBoxStyleBuilder;
use embedded_text::TextBox;
use mipidsi::models::ST7789;
use mipidsi::options::ColorInversion::Inverted;
use mipidsi::options::{Orientation, Rotation};
use mipidsi::Builder;
use rp235x_hal::block::ImageDef;
use rp235x_hal::fugit::RateExtU32;
use rp235x_hal::gpio::FunctionSpi;
use tinybmp::Bmp;

/// Tell the Boot ROM about our application
#[link_section = ".start_block"]
#[used]
pub static IMAGE_DEF: ImageDef = hal::block::ImageDef::secure_exe();


/// External high-speed crystal on the Raspberry Pi Pico 2 board is 12 MHz.
/// Adjust if your board has a different frequency
const XTAL_FREQ_HZ: u32 = 12_000_000u32;

static CIRCLE_SIZE: i32 = 65;
static ELLIPSE_SIZE: Size = Size::new(90, 65);

fn draw_shapes<T>(target: &mut T, style: PrimitiveStyle<Rgb565>) -> Result<(), T::Error>
where
    T: DrawTarget<Color = Rgb565>,
{
    Circle::new(Point::new(0, 0), CIRCLE_SIZE as u32)
        .into_styled(style)
        .draw(target)?;

    Rectangle::new(Point::new(105, 0), Size::new(64, 64))
        .into_styled(style)
        .draw(target)?;

    Triangle::new(Point::new(33, 0), Point::new(0, 64), Point::new(64, 64))
        .translate(Point::new(96 * 2 + 16, 0))
        .into_styled(style)
        .draw(target)?;

    Ellipse::new(Point::new(24, 108), ELLIPSE_SIZE)
        .into_styled(style)
        .draw(target)?;

    RoundedRectangle::new(
        Rectangle::new(Point::new(32, 0), Size::new(64, 64)),
        CornerRadii::new(Size::new(16, 16)),
    )
        .translate(Point::new(96 + 24, 108))
        .into_styled(style)
        .draw(target)
}

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
    let rst = pins.gpio20.into_push_pull_output_in_state(gpio::PinState::High);
    let miso = pins.gpio4.into_function::<FunctionSpi>();
    let mut bl = pins.gpio15.into_push_pull_output();

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
        .display_size(240, 320)
        .init(&mut delay)
        .unwrap();

    // Give display time to fully initialize
    delay.delay_ns(100_000_000u32);

    // Clear display to black
    display.clear(Rgb565::BLACK).unwrap();
    // Draw images
    let image_raw: ImageRawLE<Rgb565> = ImageRaw::new(include_bytes!("ferris.raw"), 86);
    let image: Image<_> = Image::new(&image_raw, Point::new(150, 8));
    image.draw(&mut display).unwrap();
    
    let raw_image: Bmp<Rgb565> = Bmp::from_slice(include_bytes!("rust.bmp")).unwrap();
    let image = Image::new(&raw_image, Point::new(0, 0));
    image.draw(&mut display).unwrap();

    delay.delay_ns(2_000_000_000); // 2 seconds
    display.clear(Rgb565::BLACK).unwrap();
    let bounding_box = display.bounding_box();

    let character_style = MonoTextStyleBuilder::new()
        .font(&FONT_8X13)
        .text_color(Rgb565::CSS_TOMATO)
        .build();

    let left_aligned = TextStyleBuilder::new()
        .alignment(Alignment::Left)
        .baseline(Baseline::Top)
        .build();

    let center_aligned = TextStyleBuilder::new()
        .alignment(Alignment::Center)
        .baseline(Baseline::Middle)
        .build();

    let right_aligned = TextStyleBuilder::new()
        .alignment(Alignment::Right)
        .baseline(Baseline::Bottom)
        .build();

    Text::with_text_style(
        "Left aligned text, origin top left",
        bounding_box.top_left,
        character_style,
        left_aligned,
    ).draw(&mut display).unwrap();

    Text::with_text_style(
        "Center aligned text, origin center center",
        bounding_box.center(),
        character_style,
        center_aligned,
    ).draw(&mut display).unwrap();

    Text::with_text_style(
        "Right aligned text, origin bottom right",
        bounding_box.bottom_right().unwrap(),
        character_style,
        right_aligned,
    ).draw(&mut display).unwrap();

    delay.delay_ns(2_000_000_000); // 2 seconds
    display.clear(Rgb565::BLACK).unwrap();
    let stroke = PrimitiveStyle::with_stroke(Rgb565::MAGENTA, 1);

    let stroke_off_fill_off = PrimitiveStyleBuilder::new()
        .stroke_color(Rgb565::RED)
        .stroke_width(1)
        .fill_color(Rgb565::GREEN)
        .build();

    let stroke_off_fill_on = PrimitiveStyle::with_fill(Rgb565::YELLOW);

    draw_shapes(&mut display.translated(Point::new(8, 8)), stroke).unwrap();
    draw_shapes(
        &mut display.translated(Point::new(24, 24)),
        stroke_off_fill_on,
    ).unwrap();
    draw_shapes(
        &mut display.translated(Point::new(40, 40)),
        stroke_off_fill_off,
    ).unwrap();

    delay.delay_ns(2_000_000_000); // 2 seconds
    display.clear(Rgb565::BLACK).unwrap();
    let character_style = MonoTextStyle::new(&FONT_6X10, Rgb565::CSS_AQUA);

    let textbox_style = TextBoxStyleBuilder::new()
        .alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Middle)
        .build();

    TextBox::with_textbox_style(
        "This is a\nmultiline\nHello World!",
        display.bounding_box(),
        character_style,
        textbox_style,
    ).draw(&mut display).unwrap();

    loop {
        cortex_m::asm::wfi(); // sleep infinitely
    }
}



