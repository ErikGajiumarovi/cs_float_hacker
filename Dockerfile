FROM rust:1.96-bookworm AS build
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY data ./data
RUN cargo build --release

FROM debian:bookworm-slim
RUN useradd --create-home --uid 10001 app
WORKDIR /app
COPY --from=build /app/target/release/cs-float-planner /usr/local/bin/cs-float-planner
USER app
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
CMD ["cs-float-planner"]
