- **Fix**: a workflow that finished after the list loaded stayed `Running` on the list, even
  after opening it showed `Completed`. Opening a workflow now corrects its row, going back to
  the list refreshes the statuses of the rows already listed without dropping pages or moving
  the cursor, and the info card asks for pollers only when the workflow really runs.
