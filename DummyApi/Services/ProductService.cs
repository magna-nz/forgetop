using DummyApi.Models;

namespace DummyApi.Services;

public interface IProductService
{
    IEnumerable<Product> GetAll(string? category = null);
    Product? GetById(int id);
    Product Create(CreateProductRequest request);
    bool Delete(int id);
}

public class ProductService : IProductService
{
    private readonly List<Product> _products = new()
    {
        new Product { Id = 1, Name = "Laptop", Description = "14-inch business laptop", Price = 999.99m, Stock = 50, Category = "Electronics" },
        new Product { Id = 2, Name = "Desk Chair", Description = "Ergonomic office chair", Price = 349.00m, Stock = 20, Category = "Furniture" },
        new Product { Id = 3, Name = "Notebook", Description = "A5 ruled notebook", Price = 4.99m, Stock = 200, Category = "Stationery" },
    };

    private int _nextId = 4;

    public IEnumerable<Product> GetAll(string? category = null) =>
        category is null ? _products : _products.Where(p => p.Category == category);

    public Product? GetById(int id) => _products.FirstOrDefault(p => p.Id == id);

    public Product Create(CreateProductRequest request)
    {
        var product = new Product
        {
            Id = _nextId++,
            Name = request.Name,
            Description = request.Description,
            Price = request.Price,
            Stock = request.Stock,
            Category = request.Category,
        };
        _products.Add(product);
        return product;
    }

    public bool Delete(int id)
    {
        var product = GetById(id);
        if (product is null) return false;
        _products.Remove(product);
        return true;
    }
}
