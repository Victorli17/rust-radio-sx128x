//! Sx128x Radio Driver
// Copyright 2018 Ryan Kurte

#![no_std]

use core::marker::PhantomData;
use core::convert::TryFrom;
use core::fmt::Debug;

extern crate libc;

#[macro_use]
extern crate log;

#[cfg(any(test, feature = "util"))]
#[macro_use]
extern crate std;

#[cfg(feature = "serde")]
extern crate serde;

#[cfg(feature = "util")]
#[macro_use]
extern crate structopt;

#[macro_use]
extern crate bitflags;

#[cfg(feature = "util")]
extern crate pcap_file;

extern crate failure;
use failure::{Fail};

extern crate embedded_hal as hal;
use hal::blocking::{delay};
use hal::digital::v2::{InputPin, OutputPin};
use hal::spi::{Mode as SpiMode, Phase, Polarity};
use hal::blocking::spi::{Transfer, Write, Transactional};

extern crate embedded_spi;
use embedded_spi::{Error as WrapError, wrapper::Wrapper as SpiWrapper};

extern crate radio;
pub use radio::{State as _, Interrupts as _, Channel as _};

pub mod base;

pub mod device;
pub use device::{State, Config};
use device::*;

pub mod prelude;

/// Sx128x Spi operating mode
pub const SPI_MODE: SpiMode = SpiMode {
    polarity: Polarity::IdleLow,
    phase: Phase::CaptureOnFirstTransition,
};

/// Sx128x device object
pub struct Sx128x<Base, CommsError, PinError> {
    config: Config,
    packet_type: PacketType,
    hal: Base,

    _ce: PhantomData<CommsError>, 
    _pe: PhantomData<PinError>,
}

pub const FREQ_MIN: u32 = 2_400_000_000;
pub const FREQ_MAX: u32 = 2_500_000_000;

/// Sx128x error type
#[derive(Debug, Clone, PartialEq, Fail)]
pub enum Error<CommsError: Debug + Sync + Send + 'static, PinError:  Debug + Sync + Send + 'static> {

    #[fail(display="communication error: {:?}", 0)]
    /// Communications (SPI or UART) error
    Comms(CommsError),

    #[fail(display="pin error: {:?}", 0)]
    /// Pin control error
    Pin(PinError),

    #[fail(display="transaction aborted")]
    /// Transaction aborted
    Aborted,

    #[fail(display="transaction timeout")]
    /// Timeout by device
    Timeout,

    #[fail(display="busy timeout")]
    /// Timeout awaiting busy pin de-assert
    BusyTimeout,

    #[fail(display="invalid message CRC")]
    /// CRC error on received message
    InvalidCrc,

    #[fail(display="invalid message length")]
    /// Invalid message length
    InvalidLength,
    
    #[fail(display="invalid sync word")]
    /// TODO
    InvalidSync,

    #[fail(display="transaction aborted")]
    /// TODO
    Abort,

    #[fail(display="invalid state (expected {:?} actual {:?})", 0, 1)]
    /// TODO
    InvalidState(State, State),

    #[fail(display="invalid device version (received {:?})", 0)]
    /// Radio returned an invalid device firmware version
    InvalidDevice(u16),

    #[fail(display="invalid response (received {:?})", 0)]
    /// Radio returned an invalid response
    InvalidResponse(u8),

    #[fail(display="invalid configuration")]
    /// Invalid configuration option provided
    InvalidConfiguration,
    
    #[fail(display="invalid frequency or frequency out of range")]
    /// Frequency out of range
    InvalidFrequency,

    #[fail(display="device communication failed")]
    /// No SPI communication detected
    NoComms,
}

impl <CommsError, PinError> From<WrapError<CommsError, PinError>> for Error<CommsError, PinError> where
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    fn from(e: WrapError<CommsError, PinError>) -> Self {
        match e {
            WrapError::Spi(e) => Error::Comms(e),
            WrapError::Pin(e) => Error::Pin(e),
            WrapError::Aborted => Error::Aborted,
        }
    }
}

pub type Sx128xSpi<Spi, SpiError, Output, Input, PinError, Delay> = Sx128x<SpiWrapper<Spi, SpiError, Output, Input, (), Output, PinError, Delay>, SpiError, PinError>;

impl<Spi, CommsError, Output, Input, PinError, Delay> Sx128x<SpiWrapper<Spi, CommsError, Output, Input, (), Output, PinError, Delay>, CommsError, PinError>
where
    Spi: Transfer<u8, Error = CommsError> + Write<u8, Error = CommsError> + Transactional<u8, Error = CommsError>,
    Output: OutputPin<Error = PinError>,
    Input: InputPin<Error = PinError>,
    Delay: delay::DelayMs<u32>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    /// Create an Sx128x with the provided `Spi` implementation and pins
    pub fn spi(spi: Spi, cs: Output, busy: Input, sdn: Output, delay: Delay, config: &Config) -> Result<Self, Error<CommsError, PinError>> {
        // Create SpiWrapper over spi/cs/busy
        let hal = SpiWrapper::new(spi, cs, sdn, busy, (), delay);
        // Create instance with new hal
        Self::new(hal, config)
    }
}


impl<Hal, CommsError, PinError> Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    /// Create a new Sx128x instance over a generic Hal implementation
    pub fn new(hal: Hal, config: &Config) -> Result<Self, Error<CommsError, PinError>> {

        let mut sx128x = Self::build(hal);

        debug!("Resetting device");

        // Reset IC
        sx128x.hal.reset()?;

        debug!("Checking firmware version");

        // Check communication with the radio
        let firmware_version = sx128x.firmware_version()?;
        
        if firmware_version == 0xFFFF || firmware_version == 0x0000 {
            return Err(Error::NoComms)
        } else if firmware_version != 0xA9B5 {
            warn!("Invalid firmware version! expected: 0x{:x} actual: 0x{:x}", 0xA9B5, firmware_version);
        }

        if firmware_version != 0xA9B5 && !config.skip_version_check {
            return Err(Error::InvalidDevice(firmware_version));
        }

        // TODO: do we need to calibrate things here?
        //sx128x.calibrate(CalibrationParams::default())?;

        debug!("Configuring device");

        // Configure device prior to use
        sx128x.configure(config)?;

        // Ensure state is idle
        sx128x.set_state(State::StandbyRc)?;

        Ok(sx128x)
    }

    pub fn reset(&mut self) -> Result<(), Error<CommsError, PinError>> {
        self.hal.reset()?;

        Ok(())
    }

    pub(crate) fn build(hal: Hal) -> Self {
        Sx128x { 
            config: Config::default(),
            packet_type: PacketType::None,
            hal,
            _ce: PhantomData,
            _pe: PhantomData,
        }
    }

    pub fn configure(&mut self, config: &Config) -> Result<(), Error<CommsError, PinError>> {
        // Switch to standby mode
        self.set_state(State::StandbyRc)?;

        // Check configs match
        match (&config.modem, &config.channel) {
            (Modem::LoRa(_), Channel::LoRa(_)) => (),
            (Modem::Flrc(_), Channel::Flrc(_)) => (),
            (Modem::Gfsk(_), Channel::Gfsk(_)) => (),
            _ => return Err(Error::InvalidConfiguration)
        }

        // Update regulator mode
        self.set_regulator_mode(config.regulator_mode)?;
        self.config.regulator_mode = config.regulator_mode;

        // Update modem and channel configuration
        self.set_channel(&config.channel)?;
        self.config.channel = config.channel.clone();

        self.configure_modem(&config.modem)?;
        self.config.modem = config.modem.clone();

        // Update power amplifier configuration
        self.set_power_ramp(config.pa_config.power, config.pa_config.ramp_time)?;
        self.config.pa_config = config.pa_config.clone();

        Ok(())
    }

    pub fn firmware_version(&mut self) -> Result<u16, Error<CommsError, PinError>> {
        let mut d = [0u8; 2];

        self.hal.read_regs(Registers::LrFirmwareVersionMsb as u16, &mut d)?;

        Ok((d[0] as u16) << 8 | (d[1] as u16))
    }

    pub fn set_frequency(&mut self, f: u32) -> Result<(), Error<CommsError, PinError>> {
        let c = self.config.freq_to_steps(f as f32) as u32;

        debug!("Setting frequency ({:?} MHz, {} index)", f / 1000 / 1000, c);

        let data: [u8; 3] = [
            (c >> 16) as u8,
            (c >> 8) as u8,
            (c >> 0) as u8,
        ];

        self.hal.write_cmd(Commands::SetRfFrequency as u8, &data)
    }

    pub (crate) fn set_power_ramp(&mut self, power: i8, ramp: RampTime) -> Result<(), Error<CommsError, PinError>> {
        
        if power > 13 || power < -18 {
            warn!("TX power out of range (-18 < p < 13)");
        }

        // Limit to -18 to +13 dBm
        let power = core::cmp::max(power, -18);
        let power = core::cmp::min(power, 13);
        let power_reg = (power + 18) as u8;

        debug!("Setting TX power to {} dBm {:?} ramp ({}, {})", power, ramp, power_reg, ramp as u8);
        self.config.pa_config.power = power;
        self.config.pa_config.ramp_time = ramp;

        self.hal.write_cmd(Commands::SetTxParams as u8, &[ power_reg, ramp as u8 ])
    }

    pub fn set_irq_mask(&mut self, irq: Irq) -> Result<(), Error<CommsError, PinError>> {
        debug!("Setting IRQ mask: {:?}", irq);

        let raw = irq.bits();
        self.hal.write_cmd(Commands::SetDioIrqParams as u8, &[ (raw >> 8) as u8, (raw & 0xff) as u8])
    }

    pub(crate) fn configure_modem(&mut self, config: &Modem) -> Result<(), Error<CommsError, PinError>> {
        use Modem::*;

        debug!("Setting modem config: {:?}", config);

        // First update packet type (if required)
        let packet_type = PacketType::from(config);
        if self.packet_type != packet_type {
            debug!("Setting packet type: {:?}", packet_type);
            self.hal.write_cmd(Commands::SetPacketType as u8, &[ packet_type.clone() as u8 ] )?;
            self.packet_type = packet_type;
        }

        let data = match config {
            Gfsk(c) => [c.preamble_length as u8, c.sync_word_length as u8, c.sync_word_match as u8, c.header_type as u8, c.payload_length as u8, c.crc_mode as u8, c.whitening as u8],
            LoRa(c) | Ranging(c) => [c.preamble_length as u8, c.header_type as u8, c.payload_length as u8, c.crc_mode as u8, c.invert_iq as u8, 0u8, 0u8],
            Flrc(c) => [c.preamble_length as u8, c.sync_word_length as u8, c.sync_word_match as u8, c.header_type as u8, c.payload_length as u8, c.crc_mode as u8, c.whitening as u8],
            Ble(c) => [c.connection_state as u8, c.crc_field as u8, c.packet_type as u8, c.whitening as u8, 0u8, 0u8, 0u8],
            None => [0u8; 7],
        };

        self.hal.write_cmd(Commands::SetPacketParams as u8, &data)?;

        if let Flrc(c) = config {
            if let Some(v) = c.sync_word_value {
                self.set_syncword(1, &v)?;
            }
        }

        Ok(())
    }

    pub(crate) fn get_rx_buffer_status(&mut self) -> Result<(u8, u8), Error<CommsError, PinError>> {
        use device::lora::LoRaHeader;

        let mut status = [0u8; 2];

        self.hal.read_cmd(Commands::GetRxBufferStatus as u8, &mut status)?;

        let len = match &self.config.modem {
            Modem::LoRa(c) => {
                match c.header_type {
                    LoRaHeader::Implicit => self.hal.read_reg(Registers::LrPayloadLength as u16)?,
                    LoRaHeader::Explicit => status[0],
                }
            },
            // BLE status[0] does not include 2-byte PDU header
            Modem::Ble(_) => status[0] + 2,
            _ => status[0]
        };

        let rx_buff_ptr = status[1];

        debug!("RX buffer ptr: {} len: {}", rx_buff_ptr, len);

        Ok((rx_buff_ptr, len))
    }

    
    pub(crate) fn get_packet_info(&mut self, info: &mut PacketInfo) -> Result<(), Error<CommsError, PinError>> {

        let mut data = [0u8; 5];
        self.hal.read_cmd(Commands::GetPacketStatus as u8, &mut data)?;

        info.packet_status = PacketStatus::from_bits_truncate(data[2]);
        info.tx_rx_status = TxRxStatus::from_bits_truncate(data[3]);
        info.sync_addr_status = data[4] & 0b0111;

        match self.packet_type {
            PacketType::Gfsk | PacketType::Flrc | PacketType::Ble => {
                info.rssi = -(data[1] as i16) / 2;
                let rssi_avg = -(data[0] as i16) / 2;
                debug!("Raw RSSI: {}", info.rssi);
                debug!("Average RSSI: {}", rssi_avg);
            },
            PacketType::LoRa | PacketType::Ranging => {
                info.rssi = -(data[0] as i16) / 2;
                info.snr = Some(match data[1] < 128 {
                    true => data[1] as i16 / 4,
                    false => ( data[1] as i16 - 256 ) / 4
                });
            },
            PacketType::None => unimplemented!(),
        }

        Ok(())
    }

    pub fn calibrate(&mut self, c: CalibrationParams) -> Result<(), Error<CommsError, PinError>> {
        debug!("Calibrate {:?}", c);
        self.hal.write_cmd(Commands::Calibrate as u8, &[ c.bits() ])
    }

    pub(crate) fn set_regulator_mode(&mut self, r: RegulatorMode) -> Result<(), Error<CommsError, PinError>> {
        debug!("Set regulator mode {:?}", r);
        self.hal.write_cmd(Commands::SetRegulatorMode as u8, &[ r as u8 ])
    }

    // TODO: this could got into a mode config object maybe?
    #[allow(dead_code)]
    pub(crate) fn set_auto_tx(&mut self, a: AutoTx) -> Result<(), Error<CommsError, PinError>> {
        let data = match a {
            AutoTx::Enabled(timeout_us) => {
                let compensated = timeout_us - AUTO_RX_TX_OFFSET;
                [(compensated >> 8) as u8, (compensated & 0xff) as u8]
            },
            AutoTx::Disabled => [0u8; 2],
        };
        self.hal.write_cmd(Commands::SetAutoTx as u8, &data)
    }

    pub(crate) fn set_buff_base_addr(&mut self, tx: u8, rx: u8) -> Result<(), Error<CommsError, PinError>> {
        debug!("Set buff base address (tx: {}, rx: {})", tx, rx);
        self.hal.write_cmd(Commands::SetBufferBaseAddress as u8, &[ tx, rx ])
    }

    /// Set the sychronization mode for a given index (1-3).
    /// This is 5-bytes for GFSK mode and 4-bytes for FLRC and BLE modes.
    pub fn set_syncword(&mut self, index: u8, value: &[u8]) -> Result<(), Error<CommsError, PinError>> {
        debug!("Attempting to set sync word index: {} to: {:?}", index, value);

        // Calculate sync word base address and expected length
        let (addr, len) = match (&self.packet_type, index) {
            (PacketType::Gfsk, 1) => (Registers::LrSyncWordBaseAddress1 as u16, 5),
            (PacketType::Gfsk, 2) => (Registers::LrSyncWordBaseAddress2 as u16, 5),
            (PacketType::Gfsk, 3) => (Registers::LrSyncWordBaseAddress3 as u16, 5),
            (PacketType::Flrc, 1) => (Registers::LrSyncWordBaseAddress1 as u16 + 1, 4),
            (PacketType::Flrc, 2) => (Registers::LrSyncWordBaseAddress2 as u16 + 1, 4),
            (PacketType::Flrc, 3) => (Registers::LrSyncWordBaseAddress3 as u16 + 1, 4),
            (PacketType::Ble, _) => (Registers::LrSyncWordBaseAddress1 as u16 + 1, 4),
            _ => {
                warn!("Invalid sync word configuration (mode: {:?} index: {} value: {:?}", self.config.modem, index, value);
                return Err(Error::InvalidConfiguration)
            }
        };

        // Check length is correct
        if value.len() != len {
            warn!("Incorrect sync word length for mode: {:?} (actual: {}, expected: {})", self.config.modem, value.len(), len);
            return Err(Error::InvalidConfiguration)
        }

        // Write sync word
        self.hal.write_regs(addr, value)?;

        // If we're in FLRC mode, patch to force 100% match on syncwords
        // because otherwise the 4 bit threshold is too low
        if let PacketType::Flrc = &self.packet_type {
            let r = self.hal.read_reg(Registers::LrSyncWordTolerance as u16)?;
            self.hal.write_reg(Registers::LrSyncWordTolerance as u16, r & 0xF0)?;
        }

        Ok(())
    }

}


impl<Hal, CommsError, PinError> delay::DelayMs<u32> for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    fn delay_ms(&mut self, t: u32) {
        self.hal.delay_ms(t)
    }
}

/// `radio::State` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::State for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    type State = State;
    type Error = Error<CommsError, PinError>;

    /// Fetch device state
    fn get_state(&mut self) -> Result<Self::State, Self::Error> {
        let mut d = [0u8; 1];
        self.hal.read_cmd(Commands::GetStatus as u8, &mut d)?;

        trace!("raw state: 0x{:.2x}", d[0]);

        let mode = (d[0] & 0b1110_0000) >> 5;
        let m = State::try_from(mode).map_err(|_| Error::InvalidResponse(d[0]) )?;

        let status = (d[0] & 0b0001_1100) >> 2;
        let s = CommandStatus::try_from(status).map_err(|_| Error::InvalidResponse(d[0]) )?;

        debug!("state: {:?} status: {:?}", m, s);

        Ok(m)
    }

    /// Set device state
    fn set_state(&mut self, state: Self::State) -> Result<(), Self::Error> {
        let command = match state {
            State::Tx => Commands::SetTx,
            State::Rx => Commands::SetRx,
            //State::Cad => Commands::SetCad,
            State::Fs => Commands::SetFs,
            State::StandbyRc | State::StandbyXosc => Commands::SetStandby,
            State::Sleep => Commands::SetSleep,
        };

        self.hal.write_cmd(command as u8, &[ 0u8 ])
    }
}

/// `radio::Channel` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::Channel for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    /// Channel consists of an operating frequency and packet mode
    type Channel = Channel;
    
    type Error = Error<CommsError, PinError>;

    /// Set operating channel
    fn set_channel(&mut self, ch: &Self::Channel) -> Result<(), Self::Error> {
        use Channel::*;

        debug!("Setting channel config: {:?}", ch);
        
        // Set frequency
        let freq = ch.frequency();
        if freq < FREQ_MIN || freq > FREQ_MAX {
            return Err(Error::InvalidFrequency)
        }

        self.set_frequency(freq)?;

        // First update packet type (if required)
        let packet_type = PacketType::from(ch);
        if self.packet_type != packet_type {
            self.hal.write_cmd(Commands::SetPacketType as u8, &[ packet_type.clone() as u8 ] )?;
            self.packet_type = packet_type;
        }
        
        // Then write modulation configuration
        let data = match ch {
            Gfsk(c) => [c.br_bw as u8, c.mi as u8, c.ms as u8],
            LoRa(c) | Ranging(c) => [c.sf as u8, c.bw as u8, c.cr as u8],
            Flrc(c) => [c.br_bw as u8, c.cr as u8, c.ms as u8],
            Ble(c) => [c.br_bw as u8, c.mi as u8, c.ms as u8],
        };

        self.hal.write_cmd(Commands::SetModulationParams as u8, &data)
    }
}

/// `radio::Power` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::Power for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    type Error = Error<CommsError, PinError>;

    /// Set TX power in dBm
    fn set_power(&mut self, power: i8) -> Result<(), Error<CommsError, PinError>> {
        let ramp_time = self.config.pa_config.ramp_time;
        self.set_power_ramp(power, ramp_time)
    }
}

/// `radio::Interrupts` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::Interrupts for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    type Irq = Irq;
    type Error = Error<CommsError, PinError>;

    /// Fetch (and optionally clear) current interrupts
    fn get_interrupts(&mut self, clear: bool) -> Result<Self::Irq, Self::Error> {
        let mut data = [0u8; 2];

        self.hal.read_cmd(Commands::GetIrqStatus as u8, &mut data)?;
        let irq = Irq::from_bits((data[0] as u16) << 8 | data[1] as u16).unwrap();

        if clear && !irq.is_empty() {
            self.hal.write_cmd(Commands::ClearIrqStatus as u8, &data)?;
        }

        if !irq.is_empty() {
            debug!("irq: {:?}", irq);
        }

        Ok(irq)
    }
}

/// `radio::Transmit` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::Transmit for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    type Error = Error<CommsError, PinError>;

    /// Start transmitting a packet
    fn start_transmit(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        debug!("TX start");

        // Set packet mode
        let mut modem_config = self.config.modem.clone();
        modem_config.set_payload_len(data.len() as u8);
        self.configure_modem(&modem_config)?;

        // Reset buffer addr
        self.set_buff_base_addr(0, 0)?;

        // Write data to be sent
        debug!("TX data: {:?}", data);
        self.hal.write_buff(0, data)?;
        
        // Configure ranging if used
        if PacketType::Ranging == self.packet_type {
            self.hal.write_cmd(Commands::SetRangingRole as u8, &[ RangingRole::Initiator as u8 ])?;
        }

        // Setup timout
        let config = [
            self.config.rf_timeout.step() as u8,
            (( self.config.rf_timeout.count() >> 8 ) & 0x00FF ) as u8,
            (self.config.rf_timeout.count() & 0x00FF ) as u8,
        ];
        
        // Enable IRQs
        self.set_irq_mask(Irq::TX_DONE | Irq::CRC_ERROR | Irq::RX_TX_TIMEOUT)?;

        // Enter transmit mode
        self.hal.write_cmd(Commands::SetTx as u8, &config)?;

        debug!("TX start issued");

        let state = self.get_state()?;
        debug!("State: {:?}", state);

        Ok(())
    }

    /// Check for transmit completion
    fn check_transmit(&mut self) -> Result<bool, Self::Error> {
        let irq = self.get_interrupts(true)?;
        let state = self.get_state()?;

        trace!("TX poll (irq: {:?}, state: {:?})", irq, state);

        if irq.contains(Irq::TX_DONE) {
            debug!("TX complete");
            Ok(true)
        } else if irq.contains(Irq::RX_TX_TIMEOUT) {
            debug!("TX timeout");
            Err(Error::Timeout)
        } else {
            Ok(false)
        }
    }
}

/// `radio::Receive` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::Receive for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    /// Receive info structure
    type Info = PacketInfo;

    /// RF Error object
    type Error = Error<CommsError, PinError>;

    /// Start radio in receive mode
    fn start_receive(&mut self) -> Result<(), Self::Error> {
        debug!("RX start");

        // Reset buffer addr
        self.set_buff_base_addr(0, 0)?;
        
        // Set packet mode
        // TODO: surely this should not bre required _every_ receive?
        let modem_config = self.config.modem.clone();
        self.configure_modem(&modem_config)?;

        // Configure ranging if used
        if PacketType::Ranging == self.packet_type {
            self.hal.write_cmd(Commands::SetRangingRole as u8, &[ RangingRole::Responder as u8 ])?;
        }

        // Setup timout
        let config = [
            self.config.rf_timeout.step() as u8,
            (( self.config.rf_timeout.count() >> 8 ) & 0x00FF ) as u8,
            (self.config.rf_timeout.count() & 0x00FF ) as u8,
        ];
        
        // Enable IRQs
        self.set_irq_mask(
            Irq::RX_DONE | Irq::CRC_ERROR | Irq::RX_TX_TIMEOUT
            | Irq::SYNCWORD_VALID
            | Irq::SYNCWORD_ERROR
            | Irq::HEADER_VALID
            | Irq::HEADER_ERROR
            | Irq::PREAMBLE_DETECTED
        )?;

        // Enter transmit mode
        self.hal.write_cmd(Commands::SetRx as u8, &config)?;

        let state = self.get_state()?;

        debug!("RX started (state: {:?})", state);

        Ok(())
    }

    /// Check for a received packet
    fn check_receive(&mut self, restart: bool) -> Result<bool, Self::Error> {
        let irq = self.get_interrupts(true)?;
        let mut res = Ok(false);
       
        // Process flags
        if irq.contains(Irq::CRC_ERROR) {
            debug!("RX CRC error");
            res = Err(Error::InvalidCrc);
        } else if irq.contains(Irq::RX_TX_TIMEOUT) {
            debug!("RX timeout");
            res = Err(Error::Timeout);
        } else if irq.contains(Irq::RX_DONE) {
            debug!("RX complete");
            res = Ok(true);
        }

        // Auto-restart on failure if enabled
        match (restart, res) {
            (true, Err(_)) => {
                debug!("RX restarting");
                self.start_receive()?;
                Ok(false)
            },
            (_, r) => r
        }
    }

    /// Fetch a received packet
    fn get_received<'a>(&mut self, info: &mut Self::Info, data: &'a mut [u8]) -> Result<usize, Self::Error> {
        // Fetch RX buffer information
        let (ptr, len) = self.get_rx_buffer_status()?;

        debug!("RX get received, ptr: {} len: {}", ptr, len);

        // Read from the buffer at the provided pointer
        self.hal.read_buff(ptr, &mut data[..len as usize])?;

        // Fetch related information
        self.get_packet_info(info)?;

        debug!("RX data: {:?} info: {:?}", &data[..len as usize], info);

        // Return read length
        Ok(len as usize)
    }

}

/// `radio::Rssi` implementation for the SX128x
impl<Hal, CommsError, PinError> radio::Rssi for Sx128x<Hal, CommsError, PinError>
where
    Hal: base::Hal<CommsError, PinError>,
    CommsError: Debug + Sync + Send + 'static,
    PinError: Debug + Sync + Send + 'static,
{
    type Error = Error<CommsError, PinError>;

    /// Poll for the current channel RSSI
    /// This should only be called when in receive mode
    fn poll_rssi(&mut self) -> Result<i16, Error<CommsError, PinError>> {
        let mut raw = [0u8; 1];
        self.hal.read_cmd(Commands::GetRssiInst as u8, &mut raw)?;
        Ok(-(raw[0] as i16) / 2)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Sx128x};
    use crate::base::Hal;
    use crate::device::RampTime;

    extern crate embedded_spi;
    use self::embedded_spi::mock::{Mock, Spi};

    use radio::{State as _};

    pub mod vectors;

    #[test]
    fn test_api_reset() {
        let mut m = Mock::new();
        let (spi, sdn, _busy, delay) = (m.spi(), m.pin(), m.pin(), m.delay());
        let mut radio = Sx128x::<Spi, _, _>::build(spi.clone());

        m.expect(vectors::reset(&spi, &sdn, &delay));
        radio.hal.reset().unwrap();
        m.finalise();
    }

    #[test]
    fn test_api_status() {
        let mut m = Mock::new();
        let (spi, sdn, _busy, delay) = (m.spi(), m.pin(), m.pin(), m.delay());
        let mut radio = Sx128x::<Spi, _, _>::build(spi.clone());

        m.expect(vectors::status(&spi, &sdn, &delay));
        radio.get_state().unwrap();
        m.finalise();
    }

    #[test]
    fn test_api_firmware_version() {
        let mut m = Mock::new();
        let (spi, sdn, _busy, delay) = (m.spi(), m.pin(), m.pin(), m.delay());
        let mut radio = Sx128x::<Spi, _, _>::build(spi.clone());

        m.expect(vectors::firmware_version(&spi, &sdn, &delay, 16));
        let version = radio.firmware_version().unwrap();
        m.finalise();
        assert_eq!(version, 16);
    }

    #[test]
    fn test_api_power_ramp() {
        let mut m = Mock::new();
        let (spi, sdn, _busy, delay) = (m.spi(), m.pin(), m.pin(), m.delay());
        let mut radio = Sx128x::<Spi, _, _>::build(spi.clone());

        m.expect(vectors::set_power_ramp(&spi, &sdn, &delay, 0x1f, 0xe0));
        radio.set_power_ramp(13, RampTime::Ramp20Us).unwrap();
        m.finalise();
    }
}
