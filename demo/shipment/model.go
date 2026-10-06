// Package shipment is tmprl's sample workload: a made-up freight forwarder.
//
// Nothing here ships anything. It exists to give a Temporal namespace the shape a real
// one has, so tmprl can be demonstrated and tested without anybody's production: a long
// automated workflow, a second workflow that waits on a person, several versions live at
// once, and activities that fail, retry and time out on purpose.
package shipment

// Scenario picks which of the scripted fates a shipment meets.
type Scenario string

const (
	Happy              Scenario = "happy"
	AwaitingInspection Scenario = "awaiting_inspection" // parked on a person who never answers
	CarrierDown        Scenario = "carrier_down"        // book_carrier retries for ever
	FlakyCustoms       Scenario = "flaky_customs"       // file_customs_declaration needs four tries
	BadPostcode        Scenario = "bad_postcode"        // validate_address fails for good
	SlowRating         Scenario = "slow_rating"         // rate_lanes times out
)

// Party is a made-up person or company. Every value is invented.
type Party struct {
	Name     string `json:"name"`
	Email    string `json:"email"`
	Phone    string `json:"phone"`
	Street   string `json:"street"`
	City     string `json:"city"`
	Postcode string `json:"postcode"`
	Country  string `json:"country"`
}

// Shipment is the document every activity reads from and adds to.
type Shipment struct {
	ID        string   `json:"id"`
	Scenario  Scenario `json:"scenario"`
	Version   string   `json:"version"`
	Shipper   Party    `json:"shipper"`
	Consignee Party    `json:"consignee"`
	Goods     string   `json:"goods"`
	WeightKg  int      `json:"weight_kg"`
	Pieces    int      `json:"pieces"`
	ValueUSD  int      `json:"declared_value_usd"`
	Lane      string   `json:"lane"`
}

// Result is what an activity hands back: the fields it worked out.
type Result map[string]any

// TaskInput starts the workflow that waits on a person.
type TaskInput struct {
	Type       string `json:"type"` // INSPECTION, CUSTOMS_REVIEW
	ShipmentID string `json:"shipment_id"`
	ParentID   string `json:"parent_workflow_id"`
	Assignee   string `json:"assignee"`
	// Seconds until the sample answers on the person's behalf. Zero means nobody does.
	AnswerAfterSeconds int `json:"answer_after_seconds"`
}

// Names of the two workflow types, by version: `shipment-v3` and `task_shipment-v3`.
func WorkflowType(version string) string     { return "shipment-" + version }
func TaskWorkflowType(version string) string { return "task_shipment-" + version }

const (
	SignalTaskDone     = "task-done"
	SignalTerminate    = "term-exec"
	UpdateDataSet      = "data-set"
	UpdateTaskComplete = "task-complete"
)
