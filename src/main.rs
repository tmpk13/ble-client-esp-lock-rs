// src/main.rs — BLE Peripheral with Servo (ESP32-C3)
use esp32_nimble::{uuid128, BLEAdvertisementData, BLEDevice, NimbleProperties};
use esp_idf_hal::ledc::Resolution;
use esp_idf_hal::{
    delay::FreeRtos,
    ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver},
    peripherals::Peripherals,
    units::Hertz,
};
use std::sync::{Arc, Mutex};

const SERVO_OPEN: u32 = 180;
const SERVO_CLOSE: u32 = 70;

fn set_angle(servo: &mut LedcDriver<'_>, angle: u32) {
    let max_duty = servo.get_max_duty();
    // 0.5ms..2.5ms pulse in 20ms period
    let pulse_us = 500 + (angle * 2000) / 180;
    let duty = (pulse_us * max_duty) / 20_000;
    let _ = servo.set_duty(duty);
}

fn on_msg(servo: &mut LedcDriver<'_>, incoming: &str) -> String {
    match incoming {
        "open_servo" => {
            set_angle(servo, SERVO_OPEN);
            "Opening... ".into()
        }
        "close_servo" => {
            set_angle(servo, SERVO_CLOSE);
            "Closing... ".into()
        }
        "open_close_servo" => {
            set_angle(servo, SERVO_OPEN);
            FreeRtos::delay_ms(400);
            set_angle(servo, SERVO_CLOSE);
            FreeRtos::delay_ms(400);
            set_angle(servo, SERVO_OPEN);
            "Opening... Closing... Opening... ".into()
        }
        _ => "No action taken...".into(),
    }
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;

    let timer = LedcTimerDriver::new(
        peripherals.ledc.timer0,
        &TimerConfig::default()
            .frequency(Hertz(50))
            .resolution(Resolution::Bits14),
    )?;
    let servo = Arc::new(Mutex::new(LedcDriver::new(
        peripherals.ledc.channel0,
        timer,
        peripherals.pins.gpio10,
    )?));

    set_angle(&mut servo.lock().unwrap(), SERVO_OPEN);

    let (servo_tx, servo_rx) = std::sync::mpsc::channel::<String>();
    let servo_tx = Arc::new(Mutex::new(servo_tx));

    let ble_device = BLEDevice::take();
    let server = ble_device.get_server();

    server.on_connect(|_server, desc| {
        log::info!("Connected: {:?}", desc);
    });
    server.on_disconnect(|_desc, reason| {
        log::info!("Disconnected: reason={:?}", reason);
        BLEDevice::take().get_advertising().lock().start().unwrap();
    });

    let service = server.create_service(uuid128!("921a6069-4357-4287-a9af-fd386fc0dcad"));
    let characteristic = service.lock().create_characteristic(
        uuid128!("1ad4aa0c-5cb7-4be3-9916-9c63f19c03fd"),
        NimbleProperties::READ | NimbleProperties::WRITE | NimbleProperties::NOTIFY,
    );

    characteristic.lock().on_write(move |args| {
        let incoming = std::str::from_utf8(args.recv_data())
            .unwrap_or("")
            .to_string();
        log::info!("<-- Received: {}", incoming);
        servo_tx.lock().unwrap().send(incoming).ok();
    });

    let advertising = ble_device.get_advertising();
    advertising.lock().set_data(
        BLEAdvertisementData::new()
            .name("esp-msg")
            .add_service_uuid(uuid128!("921a6069-4357-4287-a9af-fd386fc0dcad")),
    )?;
    advertising.lock().start()?;
    log::info!("Advertising...");

    loop {
        if let Ok(cmd) = servo_rx.try_recv() {
            let mut srv = servo.lock().unwrap();
            let result = on_msg(&mut srv, &cmd);
            log::info!("--> Action: {}", result);
        }
        FreeRtos::delay_ms(10);
    }
}
