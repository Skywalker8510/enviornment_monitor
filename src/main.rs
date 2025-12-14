#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;

use crate::alloc::string::ToString;
use alloc::string::String;
use bme680::{
    Bme680, FieldDataCondition, I2CAddress, IIRFilterSize, OversamplingSetting, PowerMode,
    SettingsBuilder,
};
use embassy_executor::Spawner;
use embassy_time::Duration;
use embassy_time::Timer;
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    text::Text,
};
use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_backtrace;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::I2c;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config, Spi};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{i2c::master::Config as I2CConfig, time::Rate};
use esp_println::{print, println};
use esp_radio::ble::controller::BleConnector;
use mipidsi::interface::SpiInterface;
use mipidsi::options::{Orientation, Rotation};
use mipidsi::{Builder, models::ST7789, options::ColorInversion};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.0.1

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz);
    let peripherals = esp_hal::init(config);

    let display_w = 135;
    let displat_h = 240;
    let _display_enable = Output::new(peripherals.GPIO7, Level::High, OutputConfig::default()); //pull pin high to enable display
    let mut backlight = Output::new(peripherals.GPIO45, Level::Low, OutputConfig::default()); // set medium backlight on
    let rst = Output::new(peripherals.GPIO41, Level::Low, OutputConfig::default()); // reset pin
    let cs = Output::new(peripherals.GPIO42, Level::Low, OutputConfig::default()); // keep low while driven display
    let dc = Output::new(peripherals.GPIO40, Level::Low, OutputConfig::default()); // data/clock switch
    let sck = peripherals.GPIO36;
    let miso = peripherals.GPIO37;
    let mosi = peripherals.GPIO35;

    let spi = Spi::new(peripherals.SPI2, Config::default().with_mode(Mode::_0))
        .unwrap()
        .with_sck(sck)
        .with_miso(miso)
        .with_mosi(mosi);
    let spi_device = ExclusiveDevice::new_no_delay(spi, cs).unwrap();
    let mut buffer = [0_u8; 512];
    let di = SpiInterface::new(spi_device, dc, &mut buffer);
    let mut delay = Delay::new();
    let mut display = Builder::new(ST7789, di)
        .reset_pin(rst)
        .display_size(display_w, displat_h)
        .orientation(Orientation::new().rotate(Rotation::Deg90))
        .display_offset(52, 40)
        .invert_colors(ColorInversion::Inverted)
        .init(&mut delay)
        .expect("Failed to initialize display");

    //Configure BMP688
    let i2c_config = I2CConfig::default().with_frequency(Rate::from_khz(100));
    let i2c = I2c::new(peripherals.I2C0, i2c_config)
        .unwrap()
        .with_sda(peripherals.GPIO3)
        .with_scl(peripherals.GPIO4);
    let mut delayer = Delay::new();

    let mut bmp688 = Bme680::init(i2c, &mut delayer, I2CAddress::Secondary)
        .map_err(|e| {
            log::error!("Error at bme680 init {e:?}");
        })
        .unwrap();

    let mut delay = Delay::new();

    let settings = SettingsBuilder::new()
        .with_humidity_oversampling(OversamplingSetting::OS2x)
        .with_pressure_oversampling(OversamplingSetting::OS4x)
        .with_temperature_oversampling(OversamplingSetting::OS8x)
        .with_temperature_filter(IIRFilterSize::Size3)
        .with_gas_measurement(Duration::from_millis(1500).into(), 320, 25)
        .with_temperature_offset(-2.2)
        .with_run_gas(true)
        .build();

    let profile_dur = bmp688
        .get_profile_dur(&settings.0)
        .map_err(|e| {
            log::error!("Unable to get profile dur {e:?}");
        })
        .unwrap();
    println!("Profile duration {:?}", profile_dur);
    println!("Setting sensor settings");
    bmp688
        .set_sensor_settings(&mut delayer, settings)
        .map_err(|e| {
            log::error!("Unable to apply sensor settings {e:?}");
        })
        .unwrap();
    println!("Setting forced power modes");
    bmp688
        .set_sensor_mode(&mut delayer, PowerMode::ForcedMode)
        .map_err(|e| {
            log::error!("Unable to set sensor mode {e:?}");
        })
        .unwrap();

    let sensor_settings = bmp688.get_sensor_settings(settings.1);
    println!("Sensor settings: {:?}", sensor_settings);

    delay.delay_ms(5000u32);
    let power_mode = bmp688.get_sensor_mode();
    println!("Sensor power mode: {:?}", power_mode);
    println!("Setting forced power modes");

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    let (mut _wifi_controller, _interfaces) =
        esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    let _connector = BleConnector::new(&radio_init, peripherals.BT, Default::default());

    // TODO: Spawn some tasks
    let _ = spawner;

    let _ = display.clear(Rgb565::BLACK);

    let text_style_new = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let text_style_old = MonoTextStyle::new(&FONT_10X20, Rgb565::BLACK);

    //Static Display Text
    let lines = 5; //Change based on number of lines displayed
    let max_w = display_w - 8;
    let min_w = 15;
    let space_between = ((max_w - min_w) / lines);
    print!("{}", space_between);

    let title = "Enviormental Monitor";
    let title_x = 0;
    let title_y = min_w as i32;
    let static_temp_text = "Temperature: ";
    let static_temp_x = 0;
    let static_temp_y = (max_w - space_between * 3) as i32;
    let static_pressure_text = "Pressure: ";
    let static_pressure_x = 0;
    let static_pressure_y = (max_w - space_between * 2) as i32;
    let static_humidity_text = "Humidity: ";
    let static_humidity_x = 0;
    let static_humidity_y = (max_w - space_between) as i32;
    let static_gas_text = "Air Quality: ";
    let static_gas_x = 0;
    let static_gas_y = max_w as i32;

    Text::new(title, Point::new(title_x, title_y), text_style_new)
        .draw(&mut display)
        .unwrap();
    let a = Text::new(
        static_temp_text,
        Point::new(static_temp_x, static_temp_y),
        text_style_new,
    )
    .draw(&mut display)
    .unwrap();
    let b = Text::new(
        static_pressure_text,
        Point::new(static_pressure_x, static_pressure_y),
        text_style_new,
    )
    .draw(&mut display)
    .unwrap();
    let c = Text::new(
        static_humidity_text,
        Point::new(static_humidity_x, static_humidity_y),
        text_style_new,
    )
    .draw(&mut display)
    .unwrap();
    let d = Text::new(
        static_gas_text,
        Point::new(static_gas_x, static_gas_y),
        text_style_new,
    )
    .draw(&mut display)
    .unwrap();
    println!("{:?}, {:?}, {:?}, {:?}", a, b, c, d);

    let dyn_temp_x = a.x - 5;
    let dyn_temp_y = static_temp_y;
    let dyn_pressure_x = b.x - 5;
    let dyn_pressure_y = static_pressure_y;
    let dyn_humidity_x = c.x - 5;
    let dyn_humidity_y = static_humidity_y;
    let dyn_gas_x = d.x - 5;
    let dyn_gas_y = static_gas_y;

    let mut dyn_temp_text: String;
    let mut dyn_pressure_text: String;
    let mut dyn_humidity_text: String;
    let mut dyn_gas_text: String;

    let mut old_dyn_temp_text: String = String::new();
    let mut old_dyn_pressure_text: String = String::new();
    let mut old_dyn_humidity_text: String = String::new();
    let mut old_dyn_gas_text: String = String::new();

    // Turn on backlight
    backlight.set_high();

    loop {
        bmp688
            .set_sensor_mode(&mut delayer, PowerMode::ForcedMode)
            .map_err(|e| {
                log::error!("Unable to set sensor mode {e:?}");
            })
            .unwrap();
        println!("Retrieving sensor data");
        let (data, state) = bmp688.get_sensor_data(&mut delayer).unwrap();
        println!("Sensor Data {:?}", data);

        if state == FieldDataCondition::NewData {
            if old_dyn_temp_text != data.temperature_celsius().to_string() {
                dyn_temp_text = data.temperature_celsius().to_string();
                Text::new(
                    &old_dyn_temp_text,
                    Point::new(dyn_temp_x, dyn_temp_y),
                    text_style_old,
                )
                .draw(&mut display)
                .unwrap();
                Text::new(
                    &dyn_temp_text,
                    Point::new(dyn_temp_x, dyn_temp_y),
                    text_style_new,
                )
                .draw(&mut display)
                .unwrap();
                old_dyn_temp_text = dyn_temp_text;
            }
            if old_dyn_pressure_text != data.pressure_hpa().to_string() {
                dyn_pressure_text = data.pressure_hpa().to_string();
                Text::new(
                    &old_dyn_pressure_text,
                    Point::new(dyn_pressure_x, dyn_pressure_y),
                    text_style_old,
                )
                .draw(&mut display)
                .unwrap();
                Text::new(
                    &dyn_pressure_text,
                    Point::new(dyn_pressure_x, dyn_pressure_y),
                    text_style_new,
                )
                .draw(&mut display)
                .unwrap();
                old_dyn_pressure_text = dyn_pressure_text;
            }
            if old_dyn_humidity_text != data.humidity_percent().to_string() {
                dyn_humidity_text = data.humidity_percent().to_string();
                Text::new(
                    &old_dyn_humidity_text,
                    Point::new(dyn_humidity_x, dyn_humidity_y),
                    text_style_old,
                )
                .draw(&mut display)
                .unwrap();
                Text::new(
                    &dyn_humidity_text,
                    Point::new(dyn_humidity_x, dyn_humidity_y),
                    text_style_new,
                )
                .draw(&mut display)
                .unwrap();
                old_dyn_humidity_text = dyn_humidity_text;
            }
            if old_dyn_gas_text != data.gas_resistance_ohm().to_string() {
                dyn_gas_text = data.gas_resistance_ohm().to_string();
                Text::new(
                    &old_dyn_gas_text,
                    Point::new(dyn_gas_x, dyn_gas_y),
                    text_style_old,
                )
                .draw(&mut display)
                .unwrap();
                Text::new(
                    &dyn_gas_text,
                    Point::new(dyn_gas_x, dyn_gas_y),
                    text_style_new,
                )
                .draw(&mut display)
                .unwrap();
                old_dyn_gas_text = dyn_gas_text;
            }
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}
