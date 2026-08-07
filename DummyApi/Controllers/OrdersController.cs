using Microsoft.AspNetCore.Mvc;
using DummyApi.Models;
using DummyApi.Services;

namespace DummyApi.Controllers;

[ApiController]
[Route("api/[controller]")]
public class OrdersController : ControllerBase
{
    private readonly IOrderService _orderService;

    public OrdersController(IOrderService orderService)
    {
        _orderService = orderService;
    }

    [HttpGet]
    public IActionResult GetAll([FromQuery] string? status = null) =>
        Ok(_orderService.GetAll(status));

    [HttpGet("{id}")]
    public IActionResult GetById(int id)
    {
        var order = _orderService.GetById(id);
        return order is null ? NotFound() : Ok(order);
    }

    [HttpGet("user/{userId}")]
    public IActionResult GetByUser(int userId) =>
        Ok(_orderService.GetAll().Where(o => o.UserId == userId));

    [HttpPost]
    public IActionResult Create([FromBody] CreateOrderRequest request)
    {
        var order = _orderService.Create(request);
        return CreatedAtAction(nameof(GetById), new { id = order.Id }, order);
    }

    [HttpPatch("{id}/status")]
    public IActionResult UpdateStatus(int id, [FromBody] UpdateOrderStatusRequest request)
    {
        var updated = _orderService.UpdateStatus(id, request.Status);
        return updated is null ? NotFound() : Ok(updated);
    }

    [HttpDelete("{id}")]
    public IActionResult Delete(int id)
    {
        var deleted = _orderService.Delete(id);
        return deleted ? NoContent() : NotFound();
    }
}
