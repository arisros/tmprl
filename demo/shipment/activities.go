package shipment

import (
	"context"
	"fmt"
	"hash/fnv"
	"time"

	"go.temporal.io/sdk/activity"
	"go.temporal.io/sdk/temporal"
)

// Activity names, as they appear in a workflow history.
const (
	// The framework's own steps, wrapped around every stage.
	PublishStageStartName = "PublishStageStart"
	PublishStageEndName   = "PublishStageEnd"
	StoreRevisionName     = "StoreRevision"

	ValidateAddressName        = "validate_address"
	CheckSanctionsListName     = "check_sanctions_list"
	CheckCustomerCreditName    = "check_customer_credit"
	ClassifyGoodsName          = "classify_goods"
	RateLanesName              = "rate_lanes"
	ScoreRouteRiskName         = "score_route_risk"
	CalculateDutiesName        = "calculate_duties"
	CalculateQuoteName         = "calculate_quote"
	ReserveCapacityName        = "reserve_capacity"
	BookCarrierName            = "book_carrier"
	IssueWaybillName           = "issue_waybill"
	CheckRestrictedGoodsName   = "check_restricted_goods"
	FileCustomsDeclarationName = "file_customs_declaration"
	GetClearanceStatusName     = "get_clearance_status"
	SchedulePickupName         = "schedule_pickup"
	NotifyConsigneeName        = "notify_consignee"
	UpdateTrackingName         = "update_tracking"
	CloseShipmentName          = "close_shipment"

	CreateTaskName     = "create_task"
	NotifyAssigneeName = "notify_assignee"
	CloseTaskName      = "close_task"
)

// work stands in for the call an activity would make: a pause that is the same for the
// same shipment and step, so a seeded run looks the same every time.
func work(ctx context.Context, key string, minMs, maxMs int) error {
	h := fnv.New32a()
	h.Write([]byte(key + activity.GetInfo(ctx).ActivityType.Name))
	d := time.Duration(minMs+int(h.Sum32())%(maxMs-minMs+1)) * time.Millisecond
	select {
	case <-time.After(d):
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func PublishStageStart(ctx context.Context, id, stage string) error {
	return work(ctx, id+stage, 5, 30)
}

func PublishStageEnd(ctx context.Context, id, stage string) error {
	return work(ctx, id+stage, 5, 30)
}

func StoreRevision(ctx context.Context, id string, fields Result) (Result, error) {
	if err := work(ctx, id, 10, 60); err != nil {
		return nil, err
	}
	return Result{"revision": len(fields) + 1, "stored": true}, nil
}

func ValidateAddress(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 40, 180); err != nil {
		return nil, err
	}
	if s.Scenario == BadPostcode {
		return nil, temporal.NewNonRetryableApplicationError(
			fmt.Sprintf("postcode %q is not in %s", s.Consignee.Postcode, s.Consignee.Country),
			"ValidationError", nil)
	}
	return Result{"consignee.address_valid": true, "consignee.geocode": "51.50,-0.12"}, nil
}

func CheckSanctionsList(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 60, 240); err != nil {
		return nil, err
	}
	return Result{"screening.sanctions_hit": false, "screening.list_version": "2026-10"}, nil
}

func CheckCustomerCredit(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 50, 200); err != nil {
		return nil, err
	}
	return Result{"credit.limit_usd": 25000, "credit.used_usd": s.ValueUSD / 4, "credit.ok": true}, nil
}

func ClassifyGoods(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 30, 120); err != nil {
		return nil, err
	}
	return Result{"goods.hs_code": "8471.30", "goods.dangerous": false}, nil
}

func RateLanes(ctx context.Context, s Shipment) (Result, error) {
	if s.Scenario == SlowRating {
		// Longer than the workflow allows this step, on every attempt.
		if err := work(ctx, s.ID, 6000, 6000); err != nil {
			return nil, err
		}
	}
	if err := work(ctx, s.ID, 80, 320); err != nil {
		return nil, err
	}
	return Result{"rate.lane": s.Lane, "rate.per_kg_usd": 2.35, "rate.transit_days": 6}, nil
}

func ScoreRouteRisk(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 60, 260); err != nil {
		return nil, err
	}
	return Result{"risk.score": 0.18, "risk.level": "LOW"}, nil
}

func CalculateDuties(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 30, 140); err != nil {
		return nil, err
	}
	return Result{"duties.rate": 0.045, "duties.amount_usd": float64(s.ValueUSD) * 0.045}, nil
}

func CalculateQuote(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 30, 120); err != nil {
		return nil, err
	}
	freight := float64(s.WeightKg) * 2.35
	return Result{"quote.freight_usd": freight, "quote.total_usd": freight + float64(s.ValueUSD)*0.045, "quote.valid_days": 7}, nil
}

func ReserveCapacity(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 70, 300); err != nil {
		return nil, err
	}
	return Result{"booking.reservation_id": "RSV-" + s.ID, "booking.cutoff": "18:00"}, nil
}

func BookCarrier(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 90, 380); err != nil {
		return nil, err
	}
	if s.Scenario == CarrierDown {
		return nil, temporal.NewApplicationError(
			"no capacity on lane "+s.Lane+", carrier API answered 503", "CarrierUnavailable")
	}
	return Result{"booking.carrier": "Northwind Freight", "booking.reference": "NWF" + s.ID}, nil
}

func IssueWaybill(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 40, 160); err != nil {
		return nil, err
	}
	return Result{"waybill.number": "WB-" + s.ID, "waybill.pieces": s.Pieces}, nil
}

func CheckRestrictedGoods(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 40, 180); err != nil {
		return nil, err
	}
	return Result{"customs.restricted": false}, nil
}

func FileCustomsDeclaration(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 100, 400); err != nil {
		return nil, err
	}
	if s.Scenario == FlakyCustoms && activity.GetInfo(ctx).Attempt < 4 {
		return nil, temporal.NewApplicationError(
			"customs gateway timed out after 30s", "GatewayTimeout")
	}
	return Result{"customs.declaration_id": "DEC-" + s.ID, "customs.channel": "green"}, nil
}

func GetClearanceStatus(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 50, 220); err != nil {
		return nil, err
	}
	return Result{"customs.cleared": true}, nil
}

func SchedulePickup(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 50, 200); err != nil {
		return nil, err
	}
	return Result{"pickup.window": "09:00-12:00", "pickup.driver": "D-204"}, nil
}

func NotifyConsignee(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 40, 160); err != nil {
		return nil, err
	}
	return Result{"notify.channel": "email", "notify.to": s.Consignee.Email, "notify.template": "shipment-booked-v2"}, nil
}

func UpdateTracking(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 20, 100); err != nil {
		return nil, err
	}
	return Result{"tracking.status": "IN_TRANSIT"}, nil
}

func CloseShipment(ctx context.Context, s Shipment) (Result, error) {
	if err := work(ctx, s.ID, 20, 100); err != nil {
		return nil, err
	}
	return Result{"status": "BOOKED"}, nil
}

func CreateTask(ctx context.Context, in TaskInput) (Result, error) {
	if err := work(ctx, in.ShipmentID, 30, 140); err != nil {
		return nil, err
	}
	return Result{"task.id": "TSK-" + in.ShipmentID, "task.type": in.Type, "task.assignee": in.Assignee}, nil
}

func NotifyAssignee(ctx context.Context, in TaskInput) (Result, error) {
	if err := work(ctx, in.ShipmentID, 30, 140); err != nil {
		return nil, err
	}
	return Result{"notify.to": in.Assignee, "notify.channel": "chat"}, nil
}

func CloseTask(ctx context.Context, in TaskInput, outcome string) (Result, error) {
	if err := work(ctx, in.ShipmentID, 20, 100); err != nil {
		return nil, err
	}
	return Result{"task.id": "TSK-" + in.ShipmentID, "task.outcome": outcome}, nil
}
