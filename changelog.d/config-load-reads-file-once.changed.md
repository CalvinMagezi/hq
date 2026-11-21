**Loading the config is cheaper.** Each load read `config.yaml` three times and rebuilt the list of
known keys. It now reads the file once, which matters because a chat turn loads the config several times.
