
/// LoRa mode configuration
#[derive(Clone, PartialEq, Debug)]
pub struct LoRa {
    pub spreading_factor: LoRaSpreadingFactor,
    pub bandwidth: LoRaBandwidth,
    pub coding_rate: LoRaCodingRate,
}

impl Default for LoRa {
    fn default() -> Self {
        Self {
            spreading_factor: LoRaSpreadingFactor::Sf7,
            bandwidth: LoRaBandwidth::Bw0400,
            coding_rate: LoRaCodingRate::Cr4_5,
        }
    }
}

/// Spreading factor for LoRa mode
#[derive(Clone, PartialEq, Debug)]
pub enum LoRaSpreadingFactor {
    Sf5   = 0x50,
    Sf6   = 0x60,
    Sf7   = 0x70,
    Sf8   = 0x80,
    Sf9   = 0x90,
    Sf10  = 0xA0,
    Sf11  = 0xB0,
    Sf12  = 0xC0,
}

/// Bandwidth for LoRa mode
#[derive(Clone, PartialEq, Debug)]
pub enum LoRaBandwidth {
    Bw0200  = 0x34,
    Bw0400  = 0x26,
    Bw0800  = 0x18,
    Bw1600  = 0x0A,
}

/// Coding rates for LoRa mode
#[derive(Clone, PartialEq, Debug)]
pub enum LoRaCodingRate {
    Cr4_5    = 0x01,
    Cr4_6    = 0x02,
    Cr4_7    = 0x03,
    Cr4_8    = 0x04,
    CrLI_4_5 = 0x05,
    CrLI_4_6 = 0x06,
    CrLI_4_7 = 0x07,
}

/// CRC mode for LoRa packet types
pub enum LoRaCrc {
    On = 0x20,
    Off = 0x00,
}

/// IQ mode for LoRa packet types
pub enum LoRaIq {
    Normal = 0x40,
    Inverted = 0x00,
}

