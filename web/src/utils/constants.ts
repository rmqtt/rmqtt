import axios from "axios";

const API = "http://localhost:6060/api/v1/";

export const axionsInstance = axios.create({
    baseURL: API,
    headers: {
        "Accept": "*/*",
        "Content-Type": "application/json; charset=utf-8",
    }
})