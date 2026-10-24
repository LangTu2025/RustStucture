package main

import (
	"encoding/json"
	"fmt"
	"github.com/Happy2018new/worldupgrader/blockupgrader"
	"os"
)

type Tag struct {
	Type  string `json:"type"`
	Value any    `json:"value"`
}
type State struct {
	Name       string         `json:"name"`
	Properties map[string]Tag `json:"properties"`
}
type Case struct {
	Label    string `json:"label"`
	Input    State  `json:"input"`
	Version  int32  `json:"version"`
	Expected State  `json:"expected"`
}

func integer(v any) int64 {
	n, err := v.(json.Number).Int64()
	if err != nil {
		panic(err)
	}
	return n
}
func floating(v any) float64 {
	n, err := v.(json.Number).Float64()
	if err != nil {
		panic(err)
	}
	return n
}
func untag(t Tag) any {
	switch t.Type {
	case "Byte":
		return byte(integer(t.Value))
	case "Short":
		return int16(integer(t.Value))
	case "Int":
		return int32(integer(t.Value))
	case "Long":
		return integer(t.Value)
	case "Float":
		return float32(floating(t.Value))
	case "Double":
		return floating(t.Value)
	case "String":
		return t.Value.(string)
	default:
		panic(fmt.Sprint("unsupported tag ", t.Type))
	}
}
func tags(properties map[string]any) map[string]Tag {
	out := map[string]Tag{}
	for k, v := range properties {
		switch v := v.(type) {
		case byte:
			out[k] = Tag{"Byte", int8(v)}
		case int8:
			out[k] = Tag{"Byte", v}
		case int16:
			out[k] = Tag{"Short", v}
		case int32:
			out[k] = Tag{"Int", v}
		case int64:
			out[k] = Tag{"Long", v}
		case float32:
			out[k] = Tag{"Float", v}
		case float64:
			out[k] = Tag{"Double", v}
		case string:
			out[k] = Tag{"String", v}
		default:
			panic(fmt.Sprintf("unsupported result %T", v))
		}
	}
	return out
}
func main() {
	input, err := os.Open(os.Args[1])
	if err != nil {
		panic(err)
	}
	defer input.Close()
	decoder := json.NewDecoder(input)
	decoder.UseNumber()
	var cases []Case
	if err = decoder.Decode(&cases); err != nil {
		panic(err)
	}
	for i := range cases {
		properties := map[string]any{}
		for k, v := range cases[i].Input.Properties {
			properties[k] = untag(v)
		}
		result := blockupgrader.Upgrade(blockupgrader.BlockState{Name: cases[i].Input.Name, Properties: properties, Version: cases[i].Version})
		cases[i].Expected = State{result.Name, tags(result.Properties)}
	}
	if err = json.NewEncoder(os.Stdout).Encode(cases); err != nil {
		panic(err)
	}
}
