// The sample's workers: one process polling every version's two task queues.
package main

import (
	"flag"
	"log"

	"go.temporal.io/sdk/client"
	"go.temporal.io/sdk/worker"

	"tmprl-demo/shipment"
)

func main() {
	address := flag.String("address", "127.0.0.1:7244", "Temporal frontend")
	namespace := flag.String("namespace", "default", "namespace")
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

	var workers []worker.Worker
	for _, version := range shipment.Versions {
		auto := worker.New(c, shipment.WorkflowType(version), worker.Options{})
		shipment.Register(auto, version)
		tasks := worker.New(c, shipment.TaskWorkflowType(version), worker.Options{})
		shipment.RegisterTasks(tasks, version)
		workers = append(workers, auto, tasks)
	}
	for _, w := range workers {
		if err := w.Start(); err != nil {
			log.Fatalf("start worker: %v", err)
		}
	}
	log.Printf("polling %d task queues on %s", len(workers), *address)
	<-worker.InterruptCh()
	for _, w := range workers {
		w.Stop()
	}
}
