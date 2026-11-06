package shipment

import (
	"time"

	"go.temporal.io/sdk/temporal"
	"go.temporal.io/sdk/workflow"
)

func stage(ctx workflow.Context, s Shipment, name string, doc Result, steps ...string) error {
	if err := workflow.ExecuteActivity(ctx, PublishStageStartName, s.ID, name).Get(ctx, nil); err != nil {
		return err
	}
	for _, step := range steps {
		var out Result
		if err := workflow.ExecuteActivity(ctx, step, s).Get(ctx, &out); err != nil {
			return err
		}
		for k, v := range out {
			doc[k] = v
		}
	}
	if err := workflow.ExecuteActivity(ctx, StoreRevisionName, s.ID, doc).Get(ctx, nil); err != nil {
		return err
	}
	return workflow.ExecuteActivity(ctx, PublishStageEndName, s.ID, name).Get(ctx, nil)
}

// Workflow books one shipment, from screening to dispatch.
func Workflow(ctx workflow.Context, s Shipment) (Result, error) {
	doc := Result{"id": s.ID, "status": "NEW"}
	ctx = workflow.WithActivityOptions(ctx, workflow.ActivityOptions{
		StartToCloseTimeout: 30 * time.Second,
		RetryPolicy: &temporal.RetryPolicy{
			InitialInterval:    2 * time.Second,
			BackoffCoefficient: 2,
			MaximumInterval:    time.Minute,
		},
	})

	err := workflow.SetUpdateHandler(ctx, UpdateDataSet, func(ctx workflow.Context, fields Result) (Result, error) {
		for k, v := range fields {
			doc[k] = v
		}
		return Result{"applied": len(fields)}, nil
	})
	if err != nil {
		return nil, err
	}

	if err := stage(ctx, s, "intake", doc,
		ValidateAddressName, CheckSanctionsListName, CheckCustomerCreditName, ClassifyGoodsName); err != nil {
		return nil, err
	}

	var quoteID string
	_ = workflow.SideEffect(ctx, func(workflow.Context) interface{} {
		return "Q-" + s.ID
	}).Get(&quoteID)
	doc["quote.id"] = quoteID

	rating := workflow.WithActivityOptions(ctx, workflow.ActivityOptions{
		StartToCloseTimeout: 3 * time.Second,
		RetryPolicy:         &temporal.RetryPolicy{MaximumAttempts: 2, InitialInterval: time.Second},
	})
	var rate Result
	if err := workflow.ExecuteActivity(rating, RateLanesName, s).Get(ctx, &rate); err != nil {
		return nil, err
	}
	if err := stage(ctx, s, "quote", doc,
		ScoreRouteRiskName, CalculateDutiesName, CalculateQuoteName); err != nil {
		return nil, err
	}

	if err := stage(ctx, s, "booking", doc,
		ReserveCapacityName, BookCarrierName, IssueWaybillName); err != nil {
		return nil, err
	}

	answerAfter := 4
	if s.Scenario == AwaitingInspection {
		answerAfter = 0
	}
	task := workflow.WithChildOptions(ctx, workflow.ChildWorkflowOptions{
		WorkflowID: "task-" + s.ID,
		TaskQueue:  TaskWorkflowType(s.Version),
	})
	child := workflow.ExecuteChildWorkflow(task, TaskWorkflowType(s.Version), TaskInput{
		Type:               "INSPECTION",
		ShipmentID:         s.ID,
		ParentID:           workflow.GetInfo(ctx).WorkflowExecution.ID,
		Assignee:           "inspector." + s.Consignee.Country,
		AnswerAfterSeconds: answerAfter,
	})
	var outcome string
	workflow.GetSignalChannel(ctx, SignalTaskDone).Receive(ctx, &outcome)
	doc["inspection.outcome"] = outcome
	_ = child

	if err := workflow.Sleep(ctx, time.Second); err != nil {
		return nil, err
	}

	if err := stage(ctx, s, "customs", doc,
		CheckRestrictedGoodsName, FileCustomsDeclarationName, GetClearanceStatusName); err != nil {
		return nil, err
	}
	if err := stage(ctx, s, "dispatch", doc,
		SchedulePickupName, NotifyConsigneeName, UpdateTrackingName, CloseShipmentName); err != nil {
		return nil, err
	}
	return doc, nil
}

// TaskWorkflow waits on a person, who answers through an update.
func TaskWorkflow(ctx workflow.Context, in TaskInput) (Result, error) {
	ctx = workflow.WithActivityOptions(ctx, workflow.ActivityOptions{
		StartToCloseTimeout: 30 * time.Second,
	})
	if err := workflow.ExecuteActivity(ctx, CreateTaskName, in).Get(ctx, nil); err != nil {
		return nil, err
	}
	if err := workflow.ExecuteActivity(ctx, NotifyAssigneeName, in).Get(ctx, nil); err != nil {
		return nil, err
	}

	outcome := ""
	err := workflow.SetUpdateHandler(ctx, UpdateTaskComplete, func(ctx workflow.Context, answer string) (Result, error) {
		outcome = answer
		return Result{"accepted": true}, nil
	})
	if err != nil {
		return nil, err
	}
	terminated := false
	workflow.Go(ctx, func(ctx workflow.Context) {
		workflow.GetSignalChannel(ctx, SignalTerminate).Receive(ctx, nil)
		terminated = true
	})
	if in.AnswerAfterSeconds > 0 {
		workflow.Go(ctx, func(ctx workflow.Context) {
			_ = workflow.Sleep(ctx, time.Duration(in.AnswerAfterSeconds)*time.Second)
			if outcome == "" {
				outcome = "passed"
			}
		})
	}
	if err := workflow.Await(ctx, func() bool { return outcome != "" || terminated }); err != nil {
		return nil, err
	}
	if terminated {
		outcome = "withdrawn"
	}

	var closed Result
	if err := workflow.ExecuteActivity(ctx, CloseTaskName, in, outcome).Get(ctx, &closed); err != nil {
		return nil, err
	}
	if err := workflow.SignalExternalWorkflow(ctx, in.ParentID, "", SignalTaskDone, outcome).Get(ctx, nil); err != nil {
		return nil, err
	}
	return closed, nil
}
