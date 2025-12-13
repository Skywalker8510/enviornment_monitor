#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::spi::master::{Config, Spi};
use esp_hal::spi::Mode;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use mipidsi::interface::SpiInterface;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.0.1

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::_80MHz);
    let peripherals = esp_hal::init(config);

    let _display_enable = Output::new(peripherals.GPIO7, Level::High, OutputConfig::default()); //pull pin high to enable display
    let _backlight = Output::new(peripherals.GPIO45, Level::High, OutputConfig::default()); // set medium backlight on
    let rst = Output::new(peripherals.GPIO41, Level::Low, OutputConfig::default()); // reset pin
    let cs = Output::new(peripherals.GPIO42, Level::Low, OutputConfig::default());  // keep low while driven display
    let dc = Output::new(peripherals.GPIO40, Level::Low, OutputConfig::default()); // data/clock switch
    let sck = peripherals.GPIO36;
    let miso = peripherals.GPIO37;
    let mosi = peripherals.GPIO35;

    //static SPI_BUS: static_cell::StaticCell<Mutex<NoopRawMutex, Spi<Blocking>>> = static_cell::StaticCell::new();

    let spi = Spi::new(peripherals.SPI2, Config::default().with_mode(Mode::_0)).unwrap().with_sck(sck).with_miso(miso).with_mosi(mosi);
    // let spi_bus = Mutex::new(spi);
    // let spi_bus = SPI_BUS.init(spi_bus);
    let spi_device = ExclusiveDevice::new_no_delay(spi, cs).unwrap();
    let mut buffer = [0_u8; 512];
    let di = SpiInterface::new(spi_device, dc, &mut buffer);


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

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples/src/bin
}
