# contextkernel — Python SDK

Thin HTTP client for the ContextKernel server (`ctxk serve`). See the
top-level repo at https://github.com/sudo-KNO3/contextkernel.

```python
from contextkernel import ContextKernel

kc = ContextKernel()  # defaults to http://127.0.0.1:9292
bundle = kc.query(
    task="receptor grid units",
    scope="project",
    knowledge_types=["constraint", "fact"],
    max_items=5,
)
for item in bundle.items:
    print(item.score, item.title)
```
