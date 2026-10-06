// The sample's codec server: what a client asks to read a payload.
package main

import (
	"flag"
	"log"
	"net/http"

	"go.temporal.io/sdk/converter"

	"tmprl-demo/shipment"
)

func main() {
	listen := flag.String("listen", "127.0.0.1:7799", "address to serve on")
	flag.Parse()
	handler := converter.NewPayloadCodecHTTPHandler(shipment.Codec{})
	log.Printf("codec server on http://%s", *listen)
	log.Fatal(http.ListenAndServe(*listen, handler))
}
