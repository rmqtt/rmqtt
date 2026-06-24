import axios from "axios";

const API = "http://localhost:3001/api/v1/";

export const axionsInstance = axios.create({
    baseURL: API,
    maxRedirects: 5, // Number of redirects to follow automatically
    headers: {
        "Accept": "*",
        "Access-Control-Allow-Headers" : "Content-Type",
        "Access-Control-Allow-Origin": "Origin",
    },
})