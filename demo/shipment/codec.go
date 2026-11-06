package shipment

import (
	commonpb "go.temporal.io/api/common/v1"
	"go.temporal.io/sdk/converter"
)

// Encoding marks a payload this codec has wrapped.
const Encoding = "binary/sample-xor"

// Codec XORs payloads so the cluster holds nothing readable. Not encryption, on purpose.
type Codec struct{}

const key = 0x5a

func xor(in []byte) []byte {
	out := make([]byte, len(in))
	for i, b := range in {
		out[i] = b ^ key
	}
	return out
}

func (Codec) Encode(payloads []*commonpb.Payload) ([]*commonpb.Payload, error) {
	out := make([]*commonpb.Payload, len(payloads))
	for i, p := range payloads {
		raw, err := p.Marshal()
		if err != nil {
			return nil, err
		}
		out[i] = &commonpb.Payload{
			Metadata: map[string][]byte{converter.MetadataEncoding: []byte(Encoding)},
			Data:     xor(raw),
		}
	}
	return out, nil
}

func (Codec) Decode(payloads []*commonpb.Payload) ([]*commonpb.Payload, error) {
	out := make([]*commonpb.Payload, len(payloads))
	for i, p := range payloads {
		if string(p.Metadata[converter.MetadataEncoding]) != Encoding {
			out[i] = p
			continue
		}
		var inner commonpb.Payload
		if err := inner.Unmarshal(xor(p.Data)); err != nil {
			return nil, err
		}
		out[i] = &inner
	}
	return out, nil
}

// DataConverter is the default one behind the codec.
func DataConverter() converter.DataConverter {
	return converter.NewCodecDataConverter(converter.GetDefaultDataConverter(), Codec{})
}
