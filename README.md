# comment-extract

Experimental doc comments extractor for [DataFusion](https://github.com/apache/arrow-datafusion).

## Usage

```shell
# Under arrow-datafusion project folder
comment-extract \
    --package "datafusion-expr" \
    --module-path "datafusion_expr::expr_fn" \
    --kind function
```

## Note

This tool extracts information from
rustdoc JSON output ([rfcs#2963](https://rust-lang.github.io/rfcs/2963-rustdoc-json.html)),
which requires the nightly toolchain,
but it is not necessary to compile with the nightly toolchain.
