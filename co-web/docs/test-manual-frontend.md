# Manual Frontend Test Checklist

Run through this checklist for each deployment or significant frontend change.

## Per-Variant Checks

### Variant A: Kanban + Calendar

- [ ] Board renders with columns (A Fazer, Fazendo, Feito)
- [ ] Cards display title, priority badge, and due date
- [ ] Drag and drop moves cards between columns
- [ ] Column card count updates after move
- [ ] Calendar view toggles and shows tasks by date
- [ ] Clicking a calendar date filters or highlights tasks

### Variant B: Table

- [ ] Table renders with all columns (Tarefa, Status, Prioridade, Data)
- [ ] Inline editing works for status and priority fields
- [ ] Sorting by column header works (ascending/descending toggle)
- [ ] Row selection highlights correctly
- [ ] Bulk actions (if any) apply to selected rows

### Variant C: Timeline

- [ ] Timeline renders tasks on a horizontal axis
- [ ] Tasks are positioned correctly by date
- [ ] Zoom in/out adjusts the time scale
- [ ] Hovering a task shows detail tooltip
- [ ] Dragging a task adjusts its date

## Common Checks (All Variants)

### Sidebar and Navigation

- [ ] Project list loads in sidebar
- [ ] Clicking a project switches context and loads its tasks
- [ ] Active project is visually highlighted
- [ ] "Novo Projeto" button creates a project and adds it to the list

### Task CRUD

- [ ] "Nova Tarefa" opens creation modal/form
- [ ] Required fields are validated before submission
- [ ] New task appears in the view after creation
- [ ] Editing a task persists changes after save
- [ ] Deleting a task removes it from the view
- [ ] Confirmation prompt appears before delete

### Search and Filter

- [ ] Search input filters tasks by title in real time
- [ ] Status filter shows only matching tasks
- [ ] Priority filter shows only matching tasks
- [ ] Clearing filters restores full task list

### Modal Behavior

- [ ] Modal opens centered and with backdrop overlay
- [ ] Clicking backdrop or pressing Escape closes modal
- [ ] Modal content scrolls if it overflows
- [ ] Form inputs inside modal are focusable and tabbable

### Drag Interactions (Kanban/Timeline)

- [ ] Drag cursor changes on grab
- [ ] Drop target highlights on hover
- [ ] Invalid drop targets reject the drop
- [ ] State updates immediately on successful drop

## Experiment Widget

- [ ] Widget is visible on all variants (bottom-right corner)
- [ ] Current variant label is displayed correctly
- [ ] Switching variant reloads with the new variant's UI
- [ ] Star rating (1-5) is clickable and highlights selection
- [ ] Comment text area accepts input
- [ ] "Enviar Feedback" submits and shows confirmation
- [ ] Submitting without a rating shows validation message

## Responsive Behavior

- [ ] Desktop (>1024px): Full layout with sidebar visible
- [ ] Tablet (768-1024px): Sidebar collapses to hamburger menu
- [ ] Mobile (<768px): Single-column layout, touch-friendly tap targets
- [ ] No horizontal scroll at any breakpoint
- [ ] Font sizes remain readable on all screen sizes
