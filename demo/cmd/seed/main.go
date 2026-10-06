// The sample's seeder: starts shipments with a fixed mix of fates, the same ones for the
// same seed.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"math/rand"
	"time"

	"go.temporal.io/sdk/client"

	"tmprl-demo/shipment"
)

var (
	firsts    = []string{"Mira", "Tobias", "Ines", "Kofi", "Sanna", "Rafael", "Yuki", "Leila", "Marek", "Odile"}
	lasts     = []string{"Halvorsen", "Okafor", "Lindqvist", "Marchetti", "Tanaka", "Dubois", "Novak", "Ferreira", "Aalto", "Kowalski"}
	companies = []string{"Example Optics", "Sample Ceramics", "Placeholder Tools", "Fictional Foods", "Demo Textiles", "Invented Instruments"}
	cities    = [][3]string{{"Rotterdam", "3011", "NL"}, {"Hamburg", "20095", "DE"}, {"Gdansk", "80-001", "PL"}, {"Antwerp", "2000", "BE"}, {"Valencia", "46001", "ES"}, {"Gothenburg", "41101", "SE"}}
	goods     = []string{"optical lenses", "ceramic tiles", "hand tools", "dried fruit", "woven fabric", "lab instruments", "bicycle parts"}
)

// How often each fate turns up, out of 100.
var mix = []struct {
	scenario shipment.Scenario
	weight   int
}{
	{shipment.Happy, 55},
	{shipment.AwaitingInspection, 20},
	{shipment.FlakyCustoms, 8},
	{shipment.CarrierDown, 6},
	{shipment.BadPostcode, 6},
	{shipment.SlowRating, 5},
}

func pick(r *rand.Rand) shipment.Scenario {
	n := r.Intn(100)
	for _, m := range mix {
		if n < m.weight {
			return m.scenario
		}
		n -= m.weight
	}
	return shipment.Happy
}

func party(r *rand.Rand, company bool) shipment.Party {
	city := cities[r.Intn(len(cities))]
	name := firsts[r.Intn(len(firsts))] + " " + lasts[r.Intn(len(lasts))]
	if company {
		name = companies[r.Intn(len(companies))]
	}
	return shipment.Party{
		Name:     name,
		Email:    fmt.Sprintf("contact%d@example.test", r.Intn(900)+100),
		Phone:    fmt.Sprintf("+00 555 01%02d", r.Intn(100)),
		Street:   fmt.Sprintf("%d Example Street", r.Intn(200)+1),
		City:     city[0],
		Postcode: city[1],
		Country:  city[2],
	}
}

func main() {
	address := flag.String("address", "127.0.0.1:7244", "Temporal frontend")
	namespace := flag.String("namespace", "default", "namespace")
	n := flag.Int("n", 300, "shipments to start")
	seed := flag.Int64("seed", 1, "the same seed starts the same shipments")
	rate := flag.Int("rate", 40, "shipments started a second")
	prefix := flag.String("prefix", "shp", "workflow id prefix, to seed twice without colliding")
	flag.Parse()

	c, err := client.Dial(client.Options{
		HostPort:      *address,
		Namespace:     *namespace,
		DataConverter: shipment.DataConverter(),
	})
	if err != nil {
		log.Fatalf("connect: %v", err)
	}
	defer c.Close()

	r := rand.New(rand.NewSource(*seed))
	ctx := context.Background()
	tick := time.NewTicker(time.Second / time.Duration(*rate))
	defer tick.Stop()

	var ids []string
	for i := 0; i < *n; i++ {
		<-tick.C
		version := "v3"
		if r.Intn(10) < 3 {
			version = "v2"
		}
		shipper, consignee := party(r, true), party(r, false)
		s := shipment.Shipment{
			ID:        fmt.Sprintf("%s-%05d", *prefix, i+1),
			Scenario:  pick(r),
			Version:   version,
			Shipper:   shipper,
			Consignee: consignee,
			Goods:     goods[r.Intn(len(goods))],
			WeightKg:  r.Intn(900) + 20,
			Pieces:    r.Intn(12) + 1,
			ValueUSD:  (r.Intn(180) + 5) * 100,
			Lane:      shipper.Country + "-" + consignee.Country,
		}
		if s.Scenario == shipment.BadPostcode {
			s.Consignee.Postcode = "00000"
		}
		_, err := c.ExecuteWorkflow(ctx, client.StartWorkflowOptions{
			ID:        s.ID,
			TaskQueue: shipment.WorkflowType(version),
		}, shipment.WorkflowType(version), s)
		if err != nil {
			log.Fatalf("start %s: %v", s.ID, err)
		}
		ids = append(ids, s.ID)
	}

	// A few are ended by hand, as someone on call would.
	for i, id := range ids {
		switch {
		case i%47 == 11:
			_ = c.TerminateWorkflow(ctx, id, "", "duplicate booking, superseded by "+ids[(i+1)%len(ids)])
		case i%83 == 29:
			_ = c.CancelWorkflow(ctx, id, "")
		}
	}
	log.Printf("started %d shipments on %s (seed %d)", len(ids), *address, *seed)
}
