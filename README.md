drust
=====

A simple download utility that downloads in parallel.

## Usage

    drust 0.0.1
    Kevin Darlington <kevin@outroot.com>
    Parallel download utility

    USAGE:
        drust.exe [OPTIONS] <URI>

    FLAGS:
        -h, --help       Prints help information
        -V, --version    Prints version information

    OPTIONS:
            --chunk-size <SIZE>    Specify the max chunk size downloaded at a time. (can use KB, MB, KiB, MiB etc..)
        -o, --output <FILE>        Sets the output file
            --threads <COUNT>      Max amount of threads to use to download in parallel

    ARGS:
        <URI>    Sets the URI to download from
