using Celobus.Sdk;

namespace Shop;

public sealed class ListOrders : Endpoint<ListOrdersRequest, ListOrdersResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<ListOrdersResponse> Handle(Ctx ctx, ListOrdersRequest req) =>
        Task.FromResult(new ListOrdersResponse([], "", 0));
}

public sealed class GetOrder : Endpoint<GetOrderRequest, Order>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<Order> Handle(Ctx ctx, GetOrderRequest req) => throw PluginError.NotFound("no such order");
}

public sealed class CancelOrder : Endpoint<CancelOrderRequest, Order>
{
    public override void Configure(EndpointOptions e) => e
        .Ui()
        .Roles(ShopPlugin.Clerk)
        .Idempotency(IdempotencyMode.Optional)
        .Invalidates(Topics.Orders)
        .Invalidates(Topics.Order.By(k => k.Id, (Order o) => o.Id));

    public override Task<Order> Handle(Ctx ctx, CancelOrderRequest req) => throw PluginError.NotFound("no such order");
}

public sealed class CountOpen : Endpoint<CountOpenRequest, CountResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<CountResponse> Handle(Ctx ctx, CountOpenRequest req) => Task.FromResult(new CountResponse(0));
}

public sealed class Sales : Endpoint<SalesRequest, SalesResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<SalesResponse> Handle(Ctx ctx, SalesRequest req) => Task.FromResult(new SalesResponse([], 0));
}

public sealed class TopProducts : Endpoint<TopProductsRequest, TopProductsResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<TopProductsResponse> Handle(Ctx ctx, TopProductsRequest req) =>
        Task.FromResult(new TopProductsResponse([]));
}

public sealed class GetActivity : Endpoint<GetActivityRequest, ActivityView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<ActivityView> Handle(Ctx ctx, GetActivityRequest req) =>
        Task.FromResult(new ActivityView("", "", 0, [], [], "", 0, false, "", false));
}

public sealed class GetLive : Endpoint<GetLiveRequest, LiveView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<LiveView> Handle(Ctx ctx, GetLiveRequest req) => Task.FromResult(new LiveView([], [], [], "", "", false));
}

public sealed class GetWatchers : Endpoint<GetWatchersRequest, WatchersView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<WatchersView> Handle(Ctx ctx, GetWatchersRequest req) => Task.FromResult(new WatchersView([]));
}

public sealed class PreviewNote : Endpoint<PreviewNoteRequest, PreviewView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<PreviewView> Handle(Ctx ctx, PreviewNoteRequest req) => Task.FromResult(new PreviewView(""));
}

public sealed class GetJob : Endpoint<GetJobRequest, JobView>
{
    public override void Configure(EndpointOptions e) => e.Ui().ReadOnly();

    public override Task<JobView> Handle(Ctx ctx, GetJobRequest req) => Task.FromResult(new JobView("", null));
}

public sealed class AddNote : Endpoint<AddNoteRequest, AddNoteResponse>
{
    public override void Configure(EndpointOptions e) => e.Ui().Idempotency(IdempotencyMode.Optional).Invalidates(Topics.Orders);

    public override Task<AddNoteResponse> Handle(Ctx ctx, AddNoteRequest req) => Task.FromResult(new AddNoteResponse(null));
}

public sealed class NoteAction : Endpoint<NoteActionRequest, Done>
{
    public override void Configure(EndpointOptions e) => e.Ui().Idempotency(IdempotencyMode.Optional);

    public override Task<Done> Handle(Ctx ctx, NoteActionRequest req) => Task.FromResult(new Done(false));
}

public sealed class Watch : Endpoint<WatchRequest, Done>
{
    public override void Configure(EndpointOptions e) => e.Ui().Idempotency(IdempotencyMode.Optional);

    public override Task<Done> Handle(Ctx ctx, WatchRequest req) => Task.FromResult(new Done(false));
}

public sealed class Assign : Endpoint<AssignRequest, Done>
{
    public override void Configure(EndpointOptions e) => e.Ui().Idempotency(IdempotencyMode.Optional);

    public override Task<Done> Handle(Ctx ctx, AssignRequest req) => Task.FromResult(new Done(false));
}
