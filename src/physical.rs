//! Handles the physical communication with the bus.
//!
//! This module handles the physical aspect of the communication with the bus. In particular, it correctly sets the UART communication and the associated GPIOs.

#[cfg(target_arch = "arm")]
mod hard {
    use core;

    use robus_core;
    use recv_buf;
    use Message;
    use hal::rcc;
    use ll::{TIM7 as TIMER7, USART1 as UART1, GPIOA, GPIOB, NVIC, RCC};
    use ll::interrupt::*;
    use cortex_m;

    const FREQUENCY: u32 = 48000000;

    static mut ROBUS_BAUDRATE: Option<u32> = None;

    /// Change the robus main baudrate
    ///
    /// # Arguments
    ///
    /// * `baudrate` - A u32 specifying the communication baudrate
    pub fn set_baudrate(baudrate: u32) {
        cortex_m::interrupt::free(|cs| {
            let timer = TIMER7.borrow(cs);
            let uart = UART1.borrow(cs);
            // Configure UART : baudrate
            unsafe {
                ROBUS_BAUDRATE = Some(baudrate);
            }
            uart.brr.write(|w| {
                w.div_fraction()
                    .bits((FREQUENCY / (baudrate / 2)) as u8 & 0x0F)
            });
            uart.brr.write(|w| {
                w.div_mantissa()
                    .bits(((FREQUENCY / (baudrate / 2)) >> 4) as u16)
            });
            timer
                .arr
                .modify(|_, w| w.arr().bits(((10000000 / baudrate) * 2) as u16));
        });
    }

    /// Setup the physical communication with the bus
    ///
    /// # Arguments
    ///
    /// * `baudrate` - A u32 specifying the communication baudrate
    /// * `f` - A `FnMut(u8)` reception callback - *WARNING: it will be called inside the interruption!*
    pub fn setup<F>(baudrate: u32, mut f: F)
    where
        F: FnMut(u8),
    {
        rcc::init();
        cortex_m::interrupt::free(|cs| {
            let rcc = RCC.borrow(cs);
            let gpioa = GPIOA.borrow(cs);
            let gpiob = GPIOB.borrow(cs);
            let uart = UART1.borrow(cs);

            // Enable GPIOA & GPIOB Clock
            rcc.ahbenr.modify(|_, w| w.iopaen().enabled());
            rcc.ahbenr.modify(|_, w| w.iopben().enabled());
            // Enable USART1 Clock
            rcc.apb2enr.modify(|_, w| w.usart1en().enabled());
            // Configure PTPA (PA8) et PTPB (PB13) as input with pull-up
            gpioa.moder.modify(|_, w| w.moder8().input());
            gpioa.pupdr.modify(|_, w| w.pupdr8().pull_up());
            gpiob.moder.modify(|_, w| w.moder13().input());
            gpiob.pupdr.modify(|_, w| w.pupdr13().pull_up());
            // Configure DE (PB15) /RE (PB14) pin as output
            gpiob
                .moder
                .modify(|_, w| w.moder14().output().moder15().output());
            // Default RX Enabled -> \RE = 0 & DE = 0
            gpiob.bsrr.write(|w| w.br15().set_bit().br14().set_bit());
            // Disable emitter | Enable receiver
            gpiob.bsrr.write(|w| w.br15().set_bit());
            // Configure PA9/PA10 Alternate Function 1 -> USART1
            gpioa
                .ospeedr
                .modify(|_, w| w.ospeedr9().high_speed().ospeedr10().high_speed());
            gpioa
                .pupdr
                .modify(|_, w| w.pupdr9().pull_up().pupdr10().pull_up());
            gpioa.afrh.modify(|_, w| w.afrh9().af1().afrh10().af1());
            gpioa
                .moder
                .modify(|_, w| w.moder9().alternate().moder10().alternate());
            gpioa
                .otyper
                .modify(|_, w| w.ot9().push_pull().ot10().push_pull());

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
            uart.cr3.modify(|_, w| {
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
            set_baudrate(baudrate);
            // Configure UART : Asynchronous mode
            uart.cr2
                .modify(|_, w| w.linen().disabled().clken().disabled());
            // UART1 enabled
            uart.cr1.modify(|_, w| w.ue().enabled());
        });
        unsafe {
            RECV_CB = Some(extend_lifetime(&mut f));
        }
    }

    /// Enable the Uart Interruption
    ///
    /// The callback passed to the `setup` function may now be called.
    pub fn enable_interrupt() {
        cortex_m::interrupt::free(|cs| {
            let nvic = NVIC.borrow(cs);
            nvic.enable(Interrupt::USART1);
            nvic.clear_pending(Interrupt::USART1);
        });
    }

    static mut RECV_CB: Option<&'static mut FnMut(u8)> = None;

    /// Send a byte to the UART when it's ready.
    ///
    /// *Beware, this function will block until the UART is ready to send.*
    ///
    /// # Arguments
    ///
    /// * `byte` - The u8 byte to send.
    fn send_when_ready(byte: u8) {
        cortex_m::interrupt::free(|cs| {
            // In this function we wait the transmission of the message but we don't want to block any interrupt during it.
            // This critical section line is needed, but we don't want to disable interrupt to allow other peripheral to stay alive
            // For now we just re-enable interrupt and this is a patch
            unsafe {
                cortex_m::interrupt::enable();
            }
            let gpiob = GPIOB.borrow(cs);
            let uart1 = UART1.borrow(cs);
            // TX Enabled -> \RE = 1 & DE = 1
            gpiob.bsrr.write(|w| w.bs15().set_bit().bs14().set_bit());
            while !transmit_complete(cs) {}
            uart1.tdr.modify(|_, w| w.tdr().bits(byte as u16));
        })
    }

    pub fn send(msg: &mut Message) {
        for byte in msg.to_bytes() {
            send_when_ready(byte);
        }
        // TX_LOCK unlock -> preambule idle bus during 1 byte duration
        cortex_m::interrupt::free(|cs| {
            // In this function we wait the transmission of the message but we don't want to block any interrupt during it.
            // This critical section line is needed, but we don't want to disable interrupt to allow other peripheral to stay alive
            // For now we just re-enable interrupt and this is a patch
            unsafe {
                cortex_m::interrupt::enable();
            }
            let gpiob = GPIOB.borrow(cs);
            while !transmit_complete(cs) {}
            // RX Enabled -> \RE = 0 & DE = 1
            gpiob.bsrr.write(|w| w.br15().set_bit().br14().set_bit());
            reset_timeout(cs);
            resume_timeout(cs);
        });
    }

    fn transmit_complete(cs: &cortex_m::interrupt::CriticalSection) -> bool {
        let uart1 = UART1.borrow(cs);
        if uart1.isr.read().tc().bit_is_set() {
            uart1.icr.modify(|_, w| w.tccf().clear_bit());
            true
        } else {
            false
        }
    }

    pub fn receive() {
        cortex_m::interrupt::free(|cs| {
            let uart = UART1.borrow(cs);
            if uart.isr.read().rxne().bit_is_set() {
                // we receive something, start timeout
                reset_timeout(cs);
                resume_timeout(cs);
                // get received u8
                let uart = UART1.borrow(cs);
                let uart_val = uart.rdr.read().rdr().bits();
                unsafe {
                    if let Some(ref mut cb) = RECV_CB {
                        cb(uart_val as u8);
                    }
                }
            }
        });
    }

    unsafe fn extend_lifetime<'a>(f: &'a mut FnMut(u8)) -> &'static mut FnMut(u8) {
        core::mem::transmute::<&'a mut FnMut(u8), &'static mut FnMut(u8)>(f)
    }

    /// Setup the timeout Timer
    ///
    /// The timer is used to trigger timeout event and flush the reception buffer if we read corrupted data.
    pub fn setup_timeout() {
        cortex_m::interrupt::free(|cs| {
            if let Some(ref mut baud) = unsafe { ROBUS_BAUDRATE } {
                let rcc = RCC.borrow(cs);
                let timer = TIMER7.borrow(cs);
                let nvic = NVIC.borrow(cs);

                //Enable TIM7 clock
                rcc.apb1enr.modify(|_, w| w.tim7en().enabled());

                // configure Time Out
                // Set Prescaler Register - 16 bits
                timer.psc.modify(|_, w| w.psc().bits(47));
                // Set Auto-Reload register - 32 bits -> timeout = one byte duration
                timer
                    .arr
                    .modify(|_, w| w.arr().bits(((10000000 / *baud) * 2) as u16));

                timer.cr1.modify(|_, w| w.opm().continuous());
                // Reset counter
                timer.cnt.modify(|_, w| w.cnt().bits(0));
                // Enable counter
                timer.cr1.modify(|_, w| w.cen().enabled());

                // Enable interrupt
                timer.dier.modify(|_, w| w.uie().enabled());
                // Interrupt activated
                nvic.enable(Interrupt::TIM7);
                nvic.clear_pending(Interrupt::TIM7);
            } else {
                panic!("{:?}", "No robus baudrate found");
            }
        });
    }

    pub fn pause_timeout(cs: &cortex_m::interrupt::CriticalSection) {
        let timer = TIMER7.borrow(cs);
        // Disable counter
        timer.cr1.modify(|_, w| w.cen().disabled());
    }

    pub fn reset_timeout(cs: &cortex_m::interrupt::CriticalSection) {
        let timer = TIMER7.borrow(cs);
        // Reset counter
        timer.cnt.modify(|_, w| w.cnt().bits(0));
    }

    pub fn resume_timeout(cs: &cortex_m::interrupt::CriticalSection) {
        let timer = TIMER7.borrow(cs);
        // Enable counter
        timer.cr1.modify(|_, w| w.cen().enabled());
    }

    pub fn timeout() {
        cortex_m::interrupt::free(|cs| {
            let timer = TIMER7.borrow(cs);
            // TX_LOCK release
            unsafe {
                robus_core::TX_LOCK = false;
            }
            // Clear interrupt flag
            timer.sr.modify(|_, w| w.uif().clear_bit());
            pause_timeout(cs);
            // flush message buffer
            recv_buf::flush();
        });
    }

}

#[cfg(not(target_arch = "arm"))]
mod soft {
    /// Change the robus main baudrate
    ///
    /// # Arguments
    ///
    /// * `baudrate` - A u32 specifying the communication baudrate
    pub fn set_baudrate(_baudrate: u32) {}
    /// Setup the physical communication with the bus
    ///
    /// # Arguments
    ///
    /// * `baudrate` - A u32 specifying the communication baudrate
    /// * `f` - A `FnMut(u8)` reception callback - *WARNING: it will be called inside the interruption!*
    pub fn setup<F>(_baudrate: u32, mut _f: F)
    where
        F: FnMut(u8),
    {
    }
    /// Enable the Uart Interruption
    ///
    /// The callback passed to the `setup` function may now be called.
    pub fn enable_interrupt() {}
    /// Send a byte to the UART when it's ready.
    ///
    /// *Beware, this function will block until the UART is ready to send.*
    ///
    /// # Arguments
    ///
    /// * `byte` - The u8 byte to send.
    #[allow(unused)]
    pub fn send_when_ready(_byte: u8) {}

    /// Setup the UART for debugging
    ///
    /// # Arguments
    ///
    /// * `baudrate`: the specified baudrate in `u32`
    #[allow(unused)]
    pub fn setup_debug(_baudrate: u32) {}

    /// Send a byte to the debug UART when it's ready.
    ///
    /// *Beware, this function will block until the UART is ready to send.*
    ///
    /// # Arguments
    ///
    /// * `byte` - The u8 byte to send.
    #[allow(unused)]
    pub fn debug_send_when_ready(byte: u8) {
        print!("{}", byte as char);
    }

    /// Setup the timeout Timer
    ///
    /// The timer is used to trigger timeout event and flush the reception buffer if we read corrupted data.
    pub fn setup_timeout() {}
}

#[cfg(target_arch = "arm")]
pub use self::hard::*;
#[cfg(not(target_arch = "arm"))]
pub use self::soft::*;
