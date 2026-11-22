using Celobus.Sdk;

namespace Shop;

[Message]
public enum OrderStatus { Placed, Paid, Cancelled }

[Message]
public sealed record Order(
    Guid Id, string Number, string Customer, string Email, decimal Total, string Currency, OrderStatus Status,
    DateTimeOffset PlacedAt);

[Message] public sealed record ListOrdersRequest(OrderStatus? Status, string Text, string PageToken, int PageSize);
[Message] public sealed record ListOrdersResponse(IReadOnlyList<Order> Orders, string NextPageToken, long Total);
[Message] public sealed record GetOrderRequest(Guid Id);
[Message] public sealed record CancelOrderRequest(Guid Id, string Reason);
[Message] public sealed record CountOpenRequest;
[Message] public sealed record CountResponse(long Count);

[Message] public sealed record DaySales(DateOnly Day, string Channel, decimal Total, decimal Online, decimal Retail);
[Message] public sealed record SalesRequest(string Channel, string Range);
[Message] public sealed record SalesResponse(IReadOnlyList<DaySales> Days, decimal Revenue);
[Message] public sealed record Product(string Name, decimal Revenue, int Units);
[Message] public sealed record TopProductsRequest;
[Message] public sealed record TopProductsResponse(IReadOnlyList<Product> Products);

// topic keys
[Message] public sealed record OrderKey(Guid Id);
[Message] public sealed record AllOrders;

// screen and overlay params
[Message] public sealed record OrdersParams(OrderStatus? Status);
[Message] public sealed record OrderParams(Guid Id);
[Message] public sealed record CancelParams(Guid Id);
[Message] public sealed record ReportParams(string? Product, string? Range);

// the activity screen (catalog 3) and the assign dialog
[Message] public sealed record Person(string Id, string Name, string Avatar, bool Agent, string Icon, string Engine, string Role, bool Online);
[Message] public sealed record Reaction(string Emoji, int Count, bool Mine);
[Message] public sealed record JobStep(string Kind, string Title, string Detail);
[Message] public sealed record Job(string State, string Label, string Meta, long Started, IReadOnlyList<JobStep> Rows, int Steps, string RetryLabel);
[Message] public sealed record Replies(int Count, string Last);

[Message]
public sealed record Note(
    string Id, string By, string Time, string Text, string Kind, bool Edited, bool Mine, bool Pinned, string Receipt,
    string ReceiptTone, IReadOnlyList<Reaction> Reactions, Replies? Replies, Job? Job, string JobId);

[Message] public sealed record Packing(string Job, string Agent, string Icon, string Engine, string State, long Since, long Until, string Sub, string Note);
[Message] public sealed record GetActivityRequest(string Order, string PageToken, string Tab);

[Message]
public sealed record ActivityView(
    string Title, string Status, long Ends, IReadOnlyList<Note> Notes, IReadOnlyList<Person> People, string OlderPageToken,
    int Pinned, bool Closed, string Placeholder, bool Watching);

[Message] public sealed record GetLiveRequest(string Order);
[Message] public sealed record LiveView(IReadOnlyList<Note> Notes, IReadOnlyList<Person> People, IReadOnlyList<Packing> Jobs, string Label, string Hint, bool CanStop);
[Message] public sealed record GetWatchersRequest(string Order);
[Message] public sealed record WatchersView(IReadOnlyList<Person> People);
[Message] public sealed record PreviewNoteRequest(string Order, string Text);
[Message] public sealed record PreviewView(string Hint);
[Message] public sealed record GetJobRequest(string Id, string Call);
[Message] public sealed record JobView(string Title, JobStep? Call);
[Message] public sealed record AddNoteRequest(string Order, string Text, string Reply, bool Urgent);
[Message] public sealed record AddNoteResponse(Note? Note);
[Message] public sealed record NoteActionRequest(string Note, string Action, string Text, string Emoji);
[Message] public sealed record WatchRequest(string Order, bool Watch);
[Message] public sealed record AssignRequest(string Order, string People);
[Message] public sealed record Done(bool Ok);
[Message] public sealed record ActivityParams(string? Order, string? Job);
[Message] public sealed record AssignParams(string? Order);
