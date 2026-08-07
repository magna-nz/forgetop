using Microsoft.AspNetCore.Mvc;
using DummyApi.Models;
using DummyApi.Services;

namespace DummyApi.Controllers;

[ApiController]
[Route("api/[controller]")]
public class ProductsController : ControllerBase
{
    private readonly IProductService _productService;

    public ProductsController(IProductService productService)
    {
        _productService = productService;
    }

    [HttpGet]
    public IActionResult GetAll([FromQuery] string? category = null) =>
        Ok(_productService.GetAll(category));

    [HttpGet("categories")]
    public IActionResult GetCategories() =>
        Ok(_productService.GetAll()
            .Select(p => p.Category)
            .Distinct()
            .OrderBy(c => c));

    [HttpGet("{id}")]
    public IActionResult GetById(int id)
    {
        var product = _productService.GetById(id);
        return product is null ? NotFound() : Ok(product);
    }

    [HttpPost]
    public IActionResult Create([FromBody] CreateProductRequest request)
    {
        var product = _productService.Create(request);
        return CreatedAtAction(nameof(GetById), new { id = product.Id }, product);
    }

    [HttpDelete("{id}")]
    public IActionResult Delete(int id)
    {
        var deleted = _productService.Delete(id);
        return deleted ? NoContent() : NotFound();
    }
}
