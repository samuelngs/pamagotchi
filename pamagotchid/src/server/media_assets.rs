use super::*;

pub(super) fn decode_base64(input: &str) -> anyhow::Result<Vec<u8>> {
    use base64::Engine as _;

    let data = input.split_once(',').map(|(_, data)| data).unwrap_or(input);
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data))
        .map_err(Into::into)
}

pub(super) fn media_asset_view(asset: media::MediaAsset) -> MediaAssetView {
    MediaAssetView {
        id: asset.id,
        kind: asset.kind,
        mime: asset.mime,
        filename: asset.filename,
        size: asset.size,
        sha256: asset.sha256,
    }
}
