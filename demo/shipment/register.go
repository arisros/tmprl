package shipment

import (
	"go.temporal.io/sdk/activity"
	"go.temporal.io/sdk/worker"
	"go.temporal.io/sdk/workflow"
)

// Versions live at once, the way a deploy leaves the last one running beside the new.
var Versions = []string{"v2", "v3"}

// Register wires the automated workflow and its activities to a worker.
func Register(w worker.Worker, version string) {
	w.RegisterWorkflowWithOptions(Workflow, workflow.RegisterOptions{Name: WorkflowType(version)})
	for name, fn := range map[string]any{
		PublishStageStartName:      PublishStageStart,
		PublishStageEndName:        PublishStageEnd,
		StoreRevisionName:          StoreRevision,
		ValidateAddressName:        ValidateAddress,
		CheckSanctionsListName:     CheckSanctionsList,
		CheckCustomerCreditName:    CheckCustomerCredit,
		ClassifyGoodsName:          ClassifyGoods,
		RateLanesName:              RateLanes,
		ScoreRouteRiskName:         ScoreRouteRisk,
		CalculateDutiesName:        CalculateDuties,
		CalculateQuoteName:         CalculateQuote,
		ReserveCapacityName:        ReserveCapacity,
		BookCarrierName:            BookCarrier,
		IssueWaybillName:           IssueWaybill,
		CheckRestrictedGoodsName:   CheckRestrictedGoods,
		FileCustomsDeclarationName: FileCustomsDeclaration,
		GetClearanceStatusName:     GetClearanceStatus,
		SchedulePickupName:         SchedulePickup,
		NotifyConsigneeName:        NotifyConsignee,
		UpdateTrackingName:         UpdateTracking,
		CloseShipmentName:          CloseShipment,
	} {
		w.RegisterActivityWithOptions(fn, activity.RegisterOptions{Name: name})
	}
}

// RegisterTasks wires the workflow that waits on a person.
func RegisterTasks(w worker.Worker, version string) {
	w.RegisterWorkflowWithOptions(TaskWorkflow, workflow.RegisterOptions{Name: TaskWorkflowType(version)})
	w.RegisterActivityWithOptions(CreateTask, activity.RegisterOptions{Name: CreateTaskName})
	w.RegisterActivityWithOptions(NotifyAssignee, activity.RegisterOptions{Name: NotifyAssigneeName})
	w.RegisterActivityWithOptions(CloseTask, activity.RegisterOptions{Name: CloseTaskName})
}
