package main

import (
	"context"
	"fmt"
	gosdk "github.com/promisszn/soroban-amm/packages/go-sdk"
)

func main() {
	ctx := context.Background()
	client := gosdk.NewClient("https://rpc.testnet.example")
	tx, err := client.Swap(ctx, "pool-123", 1000)
	if err != nil {
		fmt.Println("swap error:", err)
		return
	}
	fmt.Println("swap tx:", tx)
}
