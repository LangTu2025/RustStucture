package main

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"fmt"

	"os"

	"github.com/Happy2018new/worldupgrader/blockupgrader"
	"github.com/df-mc/goleveldb/leveldb"
	"github.com/df-mc/goleveldb/leveldb/opt"
	"github.com/sandertv/gophertunnel/minecraft/nbt"
)

func main() {
	dbPath := os.Args[1]
	db, err := leveldb.OpenFile(dbPath, &opt.Options{ReadOnly: true})
	if err != nil {
		panic(err)
	}
	defer db.Close()
	iterator := db.NewIterator(nil, nil)
	defer iterator.Release()
	seen := map[string]bool{}
	cases := []Case{}
	for iterator.Next() {
		key := iterator.Key()
		if len(key) != 10 || key[8] != 0x2f {
			continue
		}
		buffer := bytes.NewBuffer(iterator.Value())
		version, _ := buffer.ReadByte()
		if version != 8 && version != 9 {
			panic(fmt.Sprint("version=", version))
		}
		layers, _ := buffer.ReadByte()
		if version == 9 {
			buffer.ReadByte()
		}
		for layer := byte(0); layer < layers; layer++ {
			header, _ := buffer.ReadByte()
			bits := int(header >> 1)
			if bits != 0 {
				buffer.Next(((4096 + (32 / bits) - 1) / (32 / bits)) * 4)
			}
			count := uint32(1)
			if bits != 0 {
				binary.Read(buffer, binary.LittleEndian, &count)
			}
			for index := uint32(0); index < count; index++ {
				var record map[string]any
				if err := nbt.NewDecoderWithEncoding(buffer, nbt.LittleEndian).Decode(&record); err != nil {
					panic(err)
				}
				encoded, _ := json.Marshal(record)
				if seen[string(encoded)] {
					continue
				}
				seen[string(encoded)] = true
				name, _ := record["name"].(string)
				properties, _ := record["states"].(map[string]any)
				version, _ := record["version"].(int32)
				input := State{name, tags(properties)}
				upgraded := blockupgrader.Upgrade(blockupgrader.BlockState{Name: name, Properties: properties, Version: version})
				cases = append(cases, Case{fmt.Sprint("AKKO:", len(cases)), input, version, State{upgraded.Name, tags(upgraded.Properties)}})
			}
		}
	}
	json.NewEncoder(os.Stdout).Encode(cases)
	if err := iterator.Error(); err != nil {
		panic(err)
	}
}

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
