use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, ToNsName,
    tokio::{Listener, Stream},
    traits::tokio::Stream as _,
};

fn build_socket_name(bms_name: &str, override_name: Option<&str>) -> String {
    override_name
        .map(str::to_owned)
        .unwrap_or_else(|| format!("jkbms2mqtt-{}.sock", bms_name))
}

pub(super) fn create_listener(
    bms_name: &str,
    override_name: Option<&str>,
) -> anyhow::Result<Listener> {
    let raw = build_socket_name(bms_name, override_name);
    let name = raw.to_ns_name::<GenericNamespaced>()?;
    Ok(ListenerOptions::new().name(name).create_tokio()?)
}

pub(super) async fn connect_stream(
    bms_name: &str,
    override_name: Option<&str>,
) -> anyhow::Result<Stream> {
    let raw = build_socket_name(bms_name, override_name);
    let name = raw.to_ns_name::<GenericNamespaced>()?;
    Ok(Stream::connect(name).await?)
}
