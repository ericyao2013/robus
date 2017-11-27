use core;

use hal::rcc;
use ll::{USART1 as UART1, GPIOA, GPIOB, NVIC, RCC};
use ll::interrupt::*;
use cortex_m;

const FREQUENCY: u32 = 48000000;

static mut DATA_UART1: u16 = 0;

interrupt!(USART1, receive);

pub fn setup<F>(baudrate: u32, mut f: F)where
    F: FnMut(u8),
 {
    rcc::init();
    cortex_m::interrupt::free(|cs| {
        let rcc = RCC.borrow(cs);
        let gpioa = GPIOA.borrow(cs);
        let gpiob = GPIOB.borrow(cs);
        let uart = UART1.borrow(cs);
        let nvic = NVIC.borrow(cs);

        // Enable GPIOA & GPIOB Clock
        rcc.ahbenr.modify(|_, w| w.iopaen().enabled());
        rcc.ahbenr.modify(|_, w| w.iopben().enabled());
        // Enable USART1 Clock
        rcc.apb2enr.write(| w| w.usart1en().enabled());
        // Configure PTPA (PA8) et PTPB (PB13) as input with pull-up
        gpioa.moder.modify(|_, w| w.moder8().input());
        gpioa.pupdr.modify(|_, w| w.pupdr8().pull_up());
        gpiob.moder.modify(|_, w| w.moder13().input());
        gpiob.pupdr.modify(|_, w| w.pupdr13().pull_up());
        // Configure DE (PB15) /RE (PB14) pin as output
        gpiob.moder.modify(|_, w| { w
            .moder14().output()
            .moder15().output()
        });
        // Default RX Enabled -> \RE = 0 & DE = 0
        gpiob.bsrr.write(|w| { w
            .br15().set_bit()
            .br14().set_bit()
        });
        // Disable emitter | Enable receiver
        gpiob.bsrr.write(|w| w.br15().set_bit());
        // Configure PA9/PA10 Alternate Function 1 -> USART1
        gpioa.ospeedr.write(|w| {
            w.ospeedr9().high_speed().ospeedr10().high_speed()
        });
        gpioa.pupdr.write(
            |w| w.pupdr9().pull_up().pupdr10().pull_up(),
        );
        gpioa.afrh.write(|w| w.afrh9().af1().afrh10().af1());
        gpioa.moder.write(
            |w| w.moder9().alternate().moder10().alternate(),
        );
        gpioa.otyper.write(
            |w| w.ot9().push_pull().ot10().push_pull(),
        );

        // Configure UART : Word length
        uart.cr1.modify(|_, w| w.m()._8bits());
        // Configure UART : Parity
        uart.cr1.modify(|_, w| w.pce().disabled());
        // Configure UART : Transfert Direction - Oversampling - RX Interrupt
        uart.cr1.modify(|_, w| {
            w.te()
                .enabled()
                .re()
                .enabled()
                .over8()
                .over8()
                .rxneie()
                .enabled()
        });
        // Configure UART : 1 stop bit
        uart.cr2.modify(|_, w| w.stop()._1stop());

        // Configure UART : disable hardware flow control - Overrun interrupt
        uart.cr3.write(|w| {
            w.rtse()
                .disabled()
                .ctse()
                .disabled()
                .ctsie()
                .disabled()
                .ovrdis()
                .disabled()
        });
        // Configure UART : baudrate
        uart.brr.write(|w| {
            w.div_fraction().bits(
                (FREQUENCY / (baudrate / 2)) as u8 & 0x0F,
            )
        });
        uart.brr.write(|w| {
            w.div_mantissa().bits(
                ((FREQUENCY / (baudrate / 2)) >> 4) as u16,
            )
        });
        // Configure UART : Asynchronous mode
        uart.cr2.modify(
            |_, w| w.linen().disabled().clken().disabled(),
        );
        // UART1 enabled
        uart.cr1.modify(|_, w| w.ue().enabled());
        nvic.enable(Interrupt::USART1);
        nvic.clear_pending(Interrupt::USART1);
    });
    unsafe {
        RECV_CB = Some(extend_lifetime(&mut f));
    }
}
/*
pub fn send(byte: u8) {
    cortex_m::interrupt::free(|cs| {
        let gpiob = GPIOB.borrow(cs);
        let uart = UART1.borrow(cs);
        // Enable TX -> DE = 1 & \RE = 1
        gpiob.bsrr.write(|w| { w
            .bs15().set_bit()
            .bs14().set_bit()
        });
        uart.tdr.write(|w| w.tdr().bits(byte as u16));
        // Enable RX -> DE = 0 & \RE = 0
        gpiob.bsrr.write(|w| { w
            .br15().set_bit()
            .br14().set_bit()
        });
    })
}*/
/*
pub fn transmit_complete() -> bool {
    cortex_m::interrupt::free(|cs| {
        let uart = UART1.borrow(cs);
        if uart.isr.read().tc().bit_is_set() {
            uart.icr.write(|w| w.tccf().clear_bit());
            true
        } else {
            false
        }
    })
}*/

static mut RECV_CB: Option<&'static mut FnMut(u8)> = None;

pub fn receive_callback() {
    cortex_m::interrupt::free(|cs| {
        let uart = UART1.borrow(cs);
        unsafe {
            DATA_UART1 = uart.rdr.read().rdr().bits();
        }
    });
    unsafe {
        if let Some(ref mut cb) = RECV_CB {
            cb(DATA_UART1 as u8);
        }
    }
}

pub fn receive() {
    cortex_m::interrupt::free(|cs| {
        let uart = UART1.borrow(cs);
        if uart.isr.read().rxne().bit_is_set() {
            receive_callback();
        }
    })
}

unsafe fn extend_lifetime<'a>(f: &'a mut FnMut(u8)) -> &'static mut FnMut(u8) {
    core::mem::transmute::<&'a mut FnMut(u8), &'static mut FnMut(u8)>(f)
}