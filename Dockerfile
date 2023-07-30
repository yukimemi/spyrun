FROM rust:latest
LABEL maintainer "yukimemi <yukimemi@gmail.com>"

RUN cargo install cargo-make

CMD ["bash"]
