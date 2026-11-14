use std::collections::HashMap;
use std::pin::Pin;

use bluer::{
    Adapter, AdapterEvent, Address, DiscoveryFilter, DiscoveryTransport, Session, Uuid, UuidExt,
};
use domain::{MacAddress, Timestamp};
use futures_util::{Stream, StreamExt};
use tracing::debug;
use use_cases::{Advertisement, AdvertisementSource};

/// Advertisements read from BlueZ over D-Bus.
pub struct BluezSource<S> {
    adapter: Adapter,
    events: Pin<Box<S>>,
}

/// Powers the default adapter and starts an LE scan that reports every
/// advertisement, including repeats of unchanged data.
pub async fn start() -> bluer::Result<BluezSource<impl Stream<Item = AdapterEvent>>> {
    let adapter = Session::new().await?.default_adapter().await?;
    adapter.set_powered(true).await?;
    adapter
        .set_discovery_filter(DiscoveryFilter {
            transport: DiscoveryTransport::Le,
            duplicate_data: true,
            ..DiscoveryFilter::default()
        })
        .await?;
    let events = adapter.discover_devices_with_changes().await?;
    Ok(BluezSource {
        adapter,
        events: Box::pin(events),
    })
}

impl<S: Stream<Item = AdapterEvent> + Send> AdvertisementSource for BluezSource<S> {
    async fn next_advertisement(&mut self) -> Option<Advertisement> {
        while let Some(event) = self.events.next().await {
            if let AdapterEvent::DeviceAdded(address) = event {
                match read_advertisement(&self.adapter, address).await {
                    Ok(advertisement) => return Some(advertisement),
                    Err(error) => debug!(%address, %error, "cannot read service data"),
                }
            }
        }
        None
    }
}

async fn read_advertisement(adapter: &Adapter, address: Address) -> bluer::Result<Advertisement> {
    let data = adapter.device(address)?.service_data().await?;
    Ok(to_advertisement(
        address,
        data.unwrap_or_default(),
        Timestamp::now(),
    ))
}

/// Keeps only service data under 16-bit Bluetooth UUIDs.
fn to_advertisement(
    address: Address,
    data: HashMap<Uuid, Vec<u8>>,
    received_at: Timestamp,
) -> Advertisement {
    Advertisement {
        address: MacAddress::new(address.0),
        received_at,
        service_data: data
            .into_iter()
            .filter_map(|(uuid, bytes)| uuid.as_u16().map(|short| (short, bytes)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bluer::{Address, Uuid, UuidExt};
    use domain::{MacAddress, Timestamp};

    use super::to_advertisement;

    #[test]
    fn base_uuid_becomes_a_short_id() {
        let uuid: Uuid = "0000fcd2-0000-1000-8000-00805f9b34fb"
            .parse()
            .expect("valid uuid");
        let data = HashMap::from([(uuid, vec![1, 2])]);
        let advertisement = to_advertisement(Address::any(), data, Timestamp::from_micros(0));
        assert_eq!(
            advertisement.service_data,
            HashMap::from([(0xFCD2, vec![1, 2])])
        );
    }

    #[test]
    fn non_base_uuids_are_dropped() {
        let data = HashMap::from([
            (Uuid::from_u128(0x1234), vec![1]),
            (Uuid::from_u16(0x181A), vec![2]),
        ]);
        let advertisement = to_advertisement(Address::any(), data, Timestamp::from_micros(0));
        assert_eq!(
            advertisement.service_data,
            HashMap::from([(0x181A, vec![2])])
        );
    }

    #[test]
    fn address_is_converted() {
        let address = Address::new([0xA4, 0xC1, 0x38, 0x1C, 0x02, 0x76]);
        let advertisement = to_advertisement(address, HashMap::new(), Timestamp::from_micros(7));
        assert_eq!(
            advertisement.address,
            MacAddress::new([0xA4, 0xC1, 0x38, 0x1C, 0x02, 0x76])
        );
        assert_eq!(advertisement.received_at, Timestamp::from_micros(7));
    }
}
