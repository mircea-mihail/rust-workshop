//! Task 2

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::cell::RefCell;
use core::str::from_utf8;

use cortex_m::asm::nop;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_25, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::spi::{Blocking, Spi};
use embassy_rp::{bind_interrupts, spi, Peripheral};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Delay, Duration, Timer};
use embedded_graphics::image::{Image, ImageRawLE};
use embedded_graphics::mono_font::iso_8859_1::FONT_7X13_BOLD;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::Text;
use embedded_io_async::Write;
use futures::TryFutureExt;
use st7789::{Orientation, ST7789};
use static_cell::make_static;
use {cyw43, defmt_rtt as _, panic_probe as _};

use embassy_workshop::SPIDeviceInterface;

const DISPLAY_FREQ: u32 = 64_000_000;
const CLEAR_SCREEN_SECS: u64 = 7;

// Wifi user and password.
const WIFI_NETWORK: &str = "Wyliodrin";
const WIFI_PASSWORD: &str = "g3E2PjWy";

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<'static, Output<'static, PIN_23>, PioSpi<'static, PIN_25, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

fn toggle_level(level: Level) -> Level {
    if level == Level::Low {
        Level::High
    } else {
        Level::Low
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // ************** Display initialization - DO NOT MODIFY! *****************
    let miso = p.PIN_4;
    let display_cs = p.PIN_17;
    let mosi = p.PIN_19;
    let clk = p.PIN_18;
    let rst = p.PIN_0;
    let dc = p.PIN_16;
    let mut display_config = spi::Config::default();
    display_config.frequency = DISPLAY_FREQ;
    display_config.phase = spi::Phase::CaptureOnSecondTransition;
    display_config.polarity = spi::Polarity::IdleHigh;

    // Init SPI
    let spi: Spi<'_, _, Blocking> = Spi::new_blocking(p.SPI0, clk, mosi, miso, display_config.clone());
    let spi_bus: Mutex<NoopRawMutex, _> = Mutex::new(RefCell::new(spi));

    let display_spi = SpiDeviceWithConfig::new(&spi_bus, Output::new(display_cs, Level::High), display_config);

    let dc = Output::new(dc, Level::Low);
    let rst = Output::new(rst, Level::Low);
    let di = SPIDeviceInterface::new(display_spi, dc);

    // Init ST7789 LCD
    let mut display = ST7789::new(di, rst, 240, 240);
    display.init(&mut Delay).unwrap();
    display.set_orientation(Orientation::Portrait).unwrap();
    display.clear(Rgb565::BLACK).unwrap();
    // ************************************************************************

    // Firmware for CYW43
    let fw = include_bytes!("../../../cyw43-firmware/43439A0.bin");
    let clm = include_bytes!("../../../cyw43-firmware/43439A0_clm.bin");
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let pio_spi = PioSpi::new(&mut pio.common, pio.sm0, pio.irq0, cs, p.PIN_24, p.PIN_29, p.DMA_CH0);

    let state = make_static!(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, pio_spi, fw).await;
    unwrap!(spawner.spawn(wifi_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(10, 1, 14, 20), 22),
        dns_servers: heapless::Vec::new(),
        gateway: Some(Ipv4Address::new(10, 1, 14, 1)),
    });

    // Pseudo-random seed. Don't need to modify.
    let seed = 0x0123_4567_89ab_cdef;

    // Init network stack
    let stack = &*make_static!(Stack::new(
        net_device,
        config,
        make_static!(StackResources::<2>::new()),
        seed
    ));

    unwrap!(spawner.spawn(net_task(stack)));

    // Connect to network
    loop {
        match control.join_wpa2(WIFI_NETWORK, WIFI_PASSWORD).await {
            Ok(_) => break,
            Err(_err) => {
            }
        }
    }

    // Display Ferris image
    let raw_image_data = ImageRawLE::new(include_bytes!("../../assets/ferris.raw"), 86);
    let ferris = Image::new(&raw_image_data, Point::new(77, 77));
    ferris.draw(&mut display).unwrap();

    // Write welcome message
    let style = MonoTextStyle::new(&FONT_7X13_BOLD, Rgb565::GREEN);
    Text::new("Welcome to Rust Workshop!", Point::new(36, 190), style)
        .draw(&mut display)
        .unwrap();
    // Wait a bit
    Timer::after_secs(CLEAR_SCREEN_SECS).await;

    // Clear display
    display.clear(Rgb565::BLACK).unwrap();
    let mut line = 20;

    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];


    Text::new(".hidden baby!", Point::new(36, line), style)
        .draw(&mut display)
        .unwrap();
    line = line + 15;

    let mut green_led = Output::new(p.PIN_1, Level::Low);
    let mut red_led = Output::new(p.PIN_5, Level::Low);

    loop {
        // Check if the lines reached limit
        // and clear screen.
        // You don't need to modify this
        if line > 220  {
            line = 0;
            display.clear(Rgb565::BLACK).unwrap();
        }

        // Create a new socket
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        // Try to listen on chosen port
        // TODO 2: Modify port
        control.gpio_set(0, false).await;
        if let Err(e) = socket.accept(31337).await {
            continue;
        }

        // Turn builtin led on if the socket can listen
        control.gpio_set(0, true).await;
        red_led.set_low();
        green_led.set_high();

        let mut recv_buf = [0 as u8; 4096];
        loop {
            // TODO 3: Check how many characters it received
            let n = match socket.read(&mut recv_buf).await {
                Ok(x) => x,
                Err(_) => break,
            };

            green_led.set_low();
            red_led.set_high();

            // If the server received some characters, display it
            // You don't need to modify this
            if n > 0 {
                //control.gpio_set(0, false).await;
                Text::new(from_utf8(&recv_buf[..n]).unwrap(), Point::new(36, line), style)
                    .draw(&mut display)
                    .unwrap();
                line = line + 15;
                if line > 220  {
                    line = 0;
                    display.clear(Rgb565::BLACK).unwrap();
                }


            }

            // TODO 4: Write back to socket
            socket.write(&recv_buf[..n]).await;
        }
    }
}
